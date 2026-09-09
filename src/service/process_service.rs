//! Definition management, node dispatch, durable signals, and handler invocation.

use crate::db_service::{FluxproDbService, FluxproQueueHealth};
use crate::engine::fluxpro_handlers::FluxproHandlersContainer;
use crate::models::commands::post_signal::PostSignal;
use crate::models::commands::start_process_instance::StartProcessInstance;
use crate::models::context_map::context_map::{ContextMap, ContextMapBuilder, ContextValue};
use crate::models::execution_log::{ExecutionLogEvent, ExecutionLogLevel};
use crate::models::handle_node_result::{HandleNodeResult, HandleResultStatus};
use crate::models::id_field::IdField;
use crate::models::process_def::escalation_def::EscalationActionDef;
use crate::models::process_def::{
    Branch, GatewayKind, Next, Node, ProcessDefinition, SignalDef, StageDef, WaitFor,
};
use crate::models::process_def_error::CreateProcessError;
use crate::models::process_id::ProcessId;
use crate::models::queue::queue_task::{
    FluxproQueueTask, FluxproQueueTaskDefinition, TaskProcessOutcome,
};
use crate::models::queue::queue_task_cancel::QueueTaskCancel;
use crate::models::signal_delivery::SignalRetryPolicy;
use crate::models::version_id::VersionId;

use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::{error, info, warn};

use serde_json::json;
use sqlx::Error;
use std::sync::Arc;
use tokio::sync::Notify;
use uuid::Uuid;

mod execution;

/// Lightweight task identifier and kind for pending-work descriptions.
#[derive(Debug, Clone)]
pub struct PendingTask {
    /// Identifier of the pending task.
    pub id: String,
    /// Kind of work represented by the pending task.
    pub kind: String,
}

// Only form waits and technical waits can consume signals and expose a successor.
fn waiting_transition<'a>(
    node: &'a Node,
    signal: &IdField,
) -> Option<(&'a Node, &'a Option<Next>)> {
    let (wait_for, next) = match node {
        Node::UserTask {
            wait_for: Some(wait_for),
            next,
            ..
        } => (wait_for, next),
        Node::Wait { wait_for, next, .. } => (wait_for, next),
        _ => return None,
    };

    wait_for.signals().contains(signal).then_some((node, next))
}

// A missing or nonterminal node may still reach a compatible wait later.
fn can_wait_for_future_signal(node: Option<&Node>) -> bool {
    !matches!(node, Some(Node::End { .. }))
}

fn signal_is_declared_or_waited_for(
    signals: &[SignalDef],
    nodes: &[Node],
    signal: &IdField,
) -> bool {
    // Keep persisted process definitions created before signal declarations
    // were validated compatible when their wait node already references it.
    signals.iter().any(|defined| defined.id() == signal)
        || nodes
            .iter()
            .any(|node| waiting_transition(node, signal).is_some())
}

// Sample long-lived deferrals so retry polling does not flood execution history.
fn should_log_signal_deferral(attempts: i32) -> bool {
    attempts == 1 || attempts % 50 == 0
}

/// Application-facing workflow operations and runner execution contract.
#[async_trait]
pub trait FluxproService {
    /// Reads the unresolved incident that prevents this instance from executing.
    async fn get_open_incident(
        &self,
        _token: &IdField,
    ) -> anyhow::Result<Option<crate::db_service::incidents::ProcessIncident>> {
        anyhow::bail!("service must implement incident inspection")
    }
    /// Resumes one specific incident and wakes the runner; duplicate commands return false.
    async fn resume_instance(
        &self,
        _token: &IdField,
        _incident: uuid::Uuid,
    ) -> anyhow::Result<bool> {
        anyhow::bail!("service must implement incident recovery")
    }

    /// Validates and persists a workflow definition, returning its database ID.
    async fn create_process_def(
        &self,
        process_def: &ProcessDefinition,
    ) -> Result<ProcessId, CreateProcessError>;
    /// Persists a validated definition together with its original source text.
    async fn create_process_def_from_source(
        &self,
        process_def: &ProcessDefinition,
        source_definition: &str,
    ) -> Result<ProcessId, CreateProcessError>;
    /// Atomically publishes a new active version without replacing any stored definition.
    /// Custom service adapters fail closed until they support this operation.
    async fn publish_process_def_from_source(
        &self,
        _process_def: &ProcessDefinition,
        _source_definition: &str,
        _base_uuid: Option<uuid::Uuid>,
    ) -> Result<ProcessId, CreateProcessError> {
        Err(CreateProcessError::ValidationError(vec![
            "Atomic publication is not supported by this service".into(),
        ]))
    }
    /// Starts an instance for a definition key and queues its Start node.
    ///
    /// Requires an initial stage, a currently effective definition, and a unique
    /// business `process_id`. Returns the generated runtime token.
    async fn start_process_instance(
        &self,
        process_id: IdField,
        start_process_instance: StartProcessInstance,
    ) -> Result<IdField, CreateProcessError>;
    /// Loads a definition by key and optional version within its effective dates.
    async fn get_process(
        &self,
        process_id: IdField,
        version: Option<VersionId>,
    ) -> Result<ProcessDefinition, CreateProcessError>;
    /// Queues a declared signal using the runtime instance token as `process_id`.
    async fn post_signal(
        &self,
        process_id: IdField,
        signal: PostSignal,
    ) -> Result<(), CreateProcessError>;
    /// Resolves a business process ID and queues its signal by runtime token.
    async fn post_signal_by_process_id(
        &self,
        process_id: IdField,
        signal: PostSignal,
    ) -> Result<(), CreateProcessError>;
    /// Persists the stage and a history entry, without a reason or lifecycle callback.
    async fn assign_stage(&self, process_token: &IdField, stage: &StageDef) -> Result<(), Error>;
    /// Enqueues node entry immediately or after the supplied UTC timestamp.
    async fn queue_node(
        &self,
        process_token: &IdField,
        node: &Node,
        after: Option<DateTime<Utc>>,
    ) -> Result<(), Error>;
    /// Resolves a direct or XOR successor and enqueues it when found.
    async fn queue_next_node(&self, process_token: &IdField, next: &Next) -> anyhow::Result<()>;
    /// Admits a signal with duplicate detection and wakes the local runner.
    async fn queue_signal(
        &self,
        process_token: &IdField,
        signal: &PostSignal,
    ) -> anyhow::Result<()>;
    /// Selects the first true XOR branch using current context.
    ///
    /// Expression errors are skipped. Returns the fallback or an error if no target
    /// is selected.
    async fn select_branch_for_process(
        &self,
        process_token: &IdField,
        branches: &Vec<Branch>,
        default: &Option<IdField>,
        gateway: &GatewayKind,
    ) -> anyhow::Result<IdField>;
    /// Loads a node from the definition bound to the runtime instance token.
    async fn get_node(&self, process_token: &IdField, node_id: &IdField) -> Result<Node, Error>;
    /// Stores a typed context value under the supplied scope and key.
    async fn add_context_variable(
        &self,
        process_token: &IdField,
        scope: &str,
        var_id: &IdField,
        value: &ContextValue,
    ) -> Result<(), Error>;
    /// Merges supplied context keys into the default `_` scope.
    ///
    /// Keys absent from the supplied map remain stored; this is not a full replacement.
    async fn save_process_instance_context(
        &self,
        process_token: &IdField,
        context: &ContextMap,
    ) -> anyhow::Result<()>;
    /// Atomically leases one due task, excluding instances with another live lease.
    async fn fetch_queue_task(
        &self,
        lock_key: &IdField,
        lease_ms: u64,
    ) -> anyhow::Result<Option<FluxproQueueTaskDefinition>>;
    /// Returns ready and leased task counts with the oldest ready timestamp.
    async fn get_queue_health(&self) -> anyhow::Result<FluxproQueueHealth>;
    /// Extends a matching task lease; returns `false` when ownership is lost.
    async fn renew_queue_task_lock(
        &self,
        task_uuid: Uuid,
        lock_key: &str,
        lease_ms: u64,
    ) -> anyhow::Result<bool>;
    /// Reschedules an owned queue item and releases its lease for a later attempt.
    async fn retry_queue_task(
        &self,
        task: &FluxproQueueTaskDefinition,
        run_after: DateTime<Utc>,
    ) -> anyhow::Result<()>;
    /// Executes a leased task and atomically commits its state changes and successors.
    ///
    /// The database adapter fences commits by lease ownership and instance revision.
    /// A failed snapshot read or commit leaves the queue item recoverable. Host
    /// handlers run outside the transaction and can repeat after a failed attempt.
    /// Deferred signals and retried handler failures return `RetryScheduled`.
    async fn process_task(
        &self,
        task: FluxproQueueTaskDefinition,
        container: Arc<FluxproHandlersContainer>,
    ) -> anyhow::Result<TaskProcessOutcome>;
    /// Dispatches a node directly through the legacy low-level operations.
    ///
    /// This helper does not provide atomic queue completion or wait consumption.
    /// Use `queue_node` and the runner's `process_task` path for durable execution.
    async fn handle_node(
        &self,
        process_token: &IdField,
        node: &Node,
        container: Arc<FluxproHandlersContainer>,
    ) -> anyhow::Result<()>;
    /// Loads context and invokes a registered lifecycle handler.
    ///
    /// Returns the handler outcome without applying it. Missing registration,
    /// context lookup failures, and handler errors are propagated.
    async fn process_handler(
        &self,
        process_token: &IdField,
        handler_id: &IdField,
        args: Option<&ContextMap>,
        container: Arc<FluxproHandlersContainer>,
    ) -> anyhow::Result<HandleNodeResult>;
    /// Queues a timeout and its signal cancellation associations atomically.
    ///
    /// `on_timeout` names the successor node. `cancel_events` names signals that
    /// cancel this timeout when accepted at its originating node.
    async fn create_event_schedule(
        &self,
        process_token: &IdField,
        node: &Node,
        when: DateTime<Utc>,
        on_timeout: &IdField,
        cancel_events: Option<WaitFor>,
    ) -> anyhow::Result<()>;
    /// Loads the persisted current node; an End node remains current after completion.
    ///
    /// Returns `None` before initial node entry. Missing instances, failed database
    /// reads, and malformed node definitions return errors.
    async fn get_process_instance_current_node(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<Option<Node>>;
    /// Loads typed variables into one map keyed by name; scopes are not preserved.
    async fn get_process_instance_context(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<ContextMap>;
    /// Evaluates a Rhai boolean expression with the context bound as `ctx`.
    ///
    /// Single-quoted substrings are normalized to double-quoted strings. The engine
    /// limits execution to 50,000 operations and expression depths to 64/32. Returns
    /// `None` on parse, type, execution, or limit errors.
    async fn eval_bool_with_context(&self, when: &str, context: &ContextMap) -> Option<bool>;
    /// Loads the exact definition version bound to an instance token.
    async fn get_process_def_by_instance(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<ProcessDefinition>;
    /// Resolves a business process ID to its runtime instance token.
    async fn get_process_token_by_process_id(
        &self,
        process_id: &IdField,
    ) -> anyhow::Result<IdField>;
    /// Cancels pending timeout tasks matching the node, accepted signal, and token.
    async fn cancel_queue_task(
        &self,
        node_id: &IdField,
        signal: &IdField,
        process_token: &IdField,
    ) -> anyhow::Result<()>;
    /// Loads the declared actions for a topic in the instance's definition.
    async fn get_escalation_actions(
        &self,
        process_token: &IdField,
        escalation_id: &IdField,
    ) -> anyhow::Result<Vec<EscalationActionDef>>;
    /// Persists a structured execution event for an instance.
    async fn record_execution_log(
        &self,
        process_token: &IdField,
        event: ExecutionLogEvent,
    ) -> anyhow::Result<()>;
}

/// Workflow service backed by persistence and a local queue wakeup channel.
pub struct FluxproServiceImpl {
    db_service: Arc<dyn FluxproDbService + Send + Sync>,
    queue_wakeup: Arc<Notify>,
    signal_retry_policy: SignalRetryPolicy,
    handler_timeout: std::time::Duration,
}

impl FluxproServiceImpl {
    /// Builds a service with its own notifier and the default signal retry policy.
    pub fn new(db_service: Arc<dyn FluxproDbService + Send + Sync>) -> Self {
        Self::with_queue_wakeup(
            db_service,
            Arc::new(Notify::new()),
            SignalRetryPolicy::default(),
        )
    }

    /// Builds a service using a shared runner notifier and explicit signal policy.
    pub fn with_queue_wakeup(
        db_service: Arc<dyn FluxproDbService + Send + Sync>,
        queue_wakeup: Arc<Notify>,
        signal_retry_policy: SignalRetryPolicy,
    ) -> Self {
        Self {
            db_service,
            queue_wakeup,
            signal_retry_policy,
            handler_timeout: std::time::Duration::from_secs(300),
        }
    }

    /// Bounds each cooperative asynchronous host call; timeout suspends rather than retries.
    pub fn with_handler_timeout(mut self, duration: std::time::Duration) -> anyhow::Result<Self> {
        anyhow::ensure!(!duration.is_zero(), "handler timeout must be positive");
        self.handler_timeout = duration;
        Ok(self)
    }

    async fn log_best_effort(&self, process_token: &IdField, event: ExecutionLogEvent) {
        if let Err(error) = self
            .db_service
            .write_execution_log(process_token, &event)
            .await
        {
            // Execution logging must never become a new reason for a process to fail.
            warn!(
                "Could not persist FluxPro event {} for {}: {error:#}",
                event.event_type, process_token
            );
        }
    }

    async fn create_process_def_internal(
        &self,
        process_def: &ProcessDefinition,
        source_definition: Option<&str>,
    ) -> Result<ProcessId, CreateProcessError> {
        for node in process_def.unreachable_nodes() {
            warn!(
                "definition {} contains externally reachable-only node {}",
                process_def.key, node
            );
        }
        match process_def.compile() {
            Ok(compiled) => {
                let process_id = self
                    .db_service
                    .create_service_process_def(process_def, compiled, source_definition)
                    .await?;
                Ok(ProcessId::new(process_id.to_string()))
            }
            Err(error) => Err(error),
        }
    }
}

#[async_trait]
impl FluxproService for FluxproServiceImpl {
    async fn publish_process_def_from_source(
        &self,
        process_def: &ProcessDefinition,
        source_definition: &str,
        base_uuid: Option<uuid::Uuid>,
    ) -> Result<ProcessId, CreateProcessError> {
        if !matches!(
            process_def.status,
            crate::models::process_def::ProcessStatus::Active
        ) {
            return Err(CreateProcessError::StatusNotValid);
        }
        let compiled = process_def.compile()?;
        let uuid = self
            .db_service
            .publish_service_process_def(process_def, compiled, source_definition, base_uuid)
            .await?;
        Ok(ProcessId::new(uuid.to_string()))
    }

    async fn get_open_incident(
        &self,
        token: &IdField,
    ) -> anyhow::Result<Option<crate::db_service::incidents::ProcessIncident>> {
        self.db_service.get_open_incident(token).await
    }
    async fn resume_instance(&self, token: &IdField, incident: uuid::Uuid) -> anyhow::Result<bool> {
        let resumed = self.db_service.resume_instance(token, incident).await?;
        if resumed {
            self.queue_wakeup.notify_one();
        }
        Ok(resumed)
    }

    async fn record_execution_log(
        &self,
        process_token: &IdField,
        event: ExecutionLogEvent,
    ) -> anyhow::Result<()> {
        self.db_service
            .write_execution_log(process_token, &event)
            .await
    }

    async fn create_process_def(
        &self,
        process_def: &ProcessDefinition,
    ) -> Result<ProcessId, CreateProcessError> {
        self.create_process_def_internal(process_def, None).await
    }

    async fn create_process_def_from_source(
        &self,
        process_def: &ProcessDefinition,
        source_definition: &str,
    ) -> Result<ProcessId, CreateProcessError> {
        self.create_process_def_internal(process_def, Some(source_definition))
            .await
    }

    async fn start_process_instance(
        &self,
        process_id: IdField,
        command: StartProcessInstance,
    ) -> Result<IdField, CreateProcessError> {
        let definition = self
            .get_process(process_id, command.version.clone())
            .await?;
        let token = self
            .db_service
            .start_process_instance_atomic(&definition, command)
            .await
            .map_err(|error| {
                if let Some(sqlx::Error::Database(db)) = error.downcast_ref::<sqlx::Error>()
                    && db.constraint() == Some("process_instance_process_id_key")
                {
                    return CreateProcessError::ProcessIdNonUnique;
                }
                CreateProcessError::DbError(format!("atomic instance startup failed: {error:#}"))
            })?;
        self.queue_wakeup.notify_one();
        Ok(token)
    }

    async fn get_process(
        &self,
        process_id: IdField,
        version: Option<VersionId>,
    ) -> Result<ProcessDefinition, CreateProcessError> {
        let process_def_db = self
            .db_service
            .get_current_process_def(process_id, version)
            .await
            .map_err(|e| {
                error!("Error while getting process definition: {}", e);
                e
            })?;
        info!("process version: {}", process_def_db.version);
        Ok(process_def_db.try_into().map_err(|e| {
            error!(
                "Error while converting process definition to process id: {:?}",
                e
            );
            e
        })?)
    }

    async fn post_signal(
        &self,
        process_id: IdField,
        signal: PostSignal,
    ) -> anyhow::Result<(), CreateProcessError> {
        let process_def = self
            .get_process_def_by_instance(&process_id.clone())
            .await
            .map_err(|_| CreateProcessError::ProcessDefNotFound)?;
        if !signal_is_declared_or_waited_for(
            &process_def.signals,
            &process_def.nodes,
            &signal.signal,
        ) {
            error!("Signal {} not found", signal.signal);
            return Err(CreateProcessError::SignalNotExists);
        }

        self.queue_signal(&process_id, &signal).await.map_err(|e| {
            error!("Error while atomically queueing signal: {}", e);
            CreateProcessError::CreateSignalError(format!("{}", e))
        })?;
        Ok(())
    }

    async fn post_signal_by_process_id(
        &self,
        process_id: IdField,
        signal: PostSignal,
    ) -> Result<(), CreateProcessError> {
        info!("post_signal_by_process_id: {}", process_id);
        let process_def = self
            .get_process_token_by_process_id(&process_id)
            .await
            .map_err(|_| {
                error!(
                    "Error while getting a process token by process id: {}",
                    process_id
                );
                CreateProcessError::ProcessDefNotFound
            })?;
        self.post_signal(process_def, signal).await?;
        Ok(())
    }

    async fn assign_stage(&self, process_token: &IdField, stage: &StageDef) -> Result<(), Error> {
        let stage_uuid = self
            .db_service
            .get_stage_uuid(process_token, stage.id())
            .await
            .map_err(|e| {
                error!("Error while getting stage uuid: {}", e);
                e
            })?;
        self.db_service
            .set_process_instance_stage(process_token, stage_uuid, None)
            .await?;
        self.log_best_effort(
            process_token,
            ExecutionLogEvent::info(
                "stage.changed",
                "process_service",
                format!("Process moved to stage {}", stage.id()),
            ),
        )
        .await;
        Ok(())
    }

    async fn queue_node(
        &self,
        process_token: &IdField,
        node: &Node,
        after: Option<DateTime<Utc>>,
    ) -> Result<(), Error> {
        self.db_service
            .add_queue_task(
                FluxproQueueTask::ProcessNode {
                    process_token: process_token.clone(),
                    node: node.clone(),
                },
                after,
                None,
            )
            .await?;
        let mut event = ExecutionLogEvent::info(
            "node.queued",
            "process_service",
            format!("Node {} queued for execution", node.id()),
        );
        event.node_id = Some(node.id().to_string());
        event.details = json!({ "run_after": after });
        self.log_best_effort(process_token, event).await;
        self.queue_wakeup.notify_one();
        Ok(())
    }

    async fn queue_next_node(&self, process_token: &IdField, next: &Next) -> anyhow::Result<()> {
        match next {
            Next::To(to_node) => {
                info!("Queueing node {}", to_node);
                let node = self.get_node(process_token, to_node).await?;
                self.queue_node(process_token, &node, None).await?;
                Ok(())
            }
            Next::Routes(routes) => {
                info!("Processing routes");
                let to_node = self
                    .select_branch_for_process(
                        process_token,
                        &routes.branches,
                        &routes.default,
                        &GatewayKind::XOR,
                    )
                    .await?;
                info!("Selected node by script: {}", to_node);
                let node = self.get_node(process_token, &to_node).await?;
                self.queue_node(process_token, &node, None).await?;
                Ok(())
            }
        }
    }

    async fn queue_signal(
        &self,
        process_token: &IdField,
        signal: &PostSignal,
    ) -> anyhow::Result<()> {
        if !self
            .db_service
            .enqueue_signal_if_new(process_token, signal)
            .await?
        {
            warn!("Signal {} already sent; skipping duplicate", signal.signal);
            return Ok(());
        }
        let mut event = ExecutionLogEvent::info(
            "signal.queued",
            "process_service",
            format!("Signal {} queued", signal.signal),
        );
        event.details = json!({ "signal": signal.signal, "context": signal.context });
        self.log_best_effort(process_token, event).await;
        self.queue_wakeup.notify_one();
        Ok(())
    }

    async fn select_branch_for_process(
        &self,
        process_token: &IdField,
        branches: &Vec<Branch>,
        default: &Option<IdField>,
        gateway: &GatewayKind,
    ) -> anyhow::Result<IdField> {
        let context = self.get_process_instance_context(process_token).await?;
        match gateway {
            GatewayKind::XOR => {
                for b in branches {
                    info!("Evaluating branch: {}", b.when);
                    if crate::service::expressions::evaluate_condition(&b.when, &context)? {
                        info!("Branch evaluated to true");
                        return Ok(b.next.clone());
                    }
                }
            }
        }

        info!("No branch evaluated to true");
        if let Some(def) = default.clone() {
            Ok(def)
        } else {
            Err(anyhow::anyhow!("No default branch found"))
        }
    }

    async fn get_node(&self, process_token: &IdField, node_id: &IdField) -> Result<Node, Error> {
        Ok(self
            .db_service
            .get_process_def_node(process_token, node_id)
            .await?)
    }

    async fn add_context_variable(
        &self,
        process_token: &IdField,
        scope: &str,
        var_id: &IdField,
        value: &ContextValue,
    ) -> Result<(), Error> {
        self.db_service
            .add_context_variable(process_token, scope, var_id, value)
            .await
            .map_err(|e| {
                error!("Error while adding context variable: {}", e);
                e
            })?;
        Ok(())
    }

    async fn save_process_instance_context(
        &self,
        process_token: &IdField,
        context: &ContextMap,
    ) -> anyhow::Result<()> {
        self.db_service
            .save_process_instance_context(process_token, context)
            .await?;
        Ok(())
    }

    async fn fetch_queue_task(
        &self,
        lock_key: &IdField,
        lease_ms: u64,
    ) -> anyhow::Result<Option<FluxproQueueTaskDefinition>> {
        self.db_service.fetch_queue_task(lock_key, lease_ms).await
    }

    async fn get_queue_health(&self) -> anyhow::Result<FluxproQueueHealth> {
        self.db_service.get_queue_health().await
    }

    async fn renew_queue_task_lock(
        &self,
        task_uuid: Uuid,
        lock_key: &str,
        lease_ms: u64,
    ) -> anyhow::Result<bool> {
        self.db_service
            .renew_queue_task_lock(task_uuid, lock_key, lease_ms)
            .await
    }

    async fn retry_queue_task(
        &self,
        task: &FluxproQueueTaskDefinition,
        run_after: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        self.db_service.retry_queue_task(task, run_after).await?;
        self.queue_wakeup.notify_one();
        Ok(())
    }

    async fn process_task(
        &self,
        task: FluxproQueueTaskDefinition,
        container: Arc<FluxproHandlersContainer>,
    ) -> anyhow::Result<TaskProcessOutcome> {
        execution::process(self, task, container).await
    }

    async fn handle_node(
        &self,
        process_token: &IdField,
        node: &Node,
        container: Arc<FluxproHandlersContainer>,
    ) -> anyhow::Result<()> {
        let mut entered = ExecutionLogEvent::info(
            "node.handling_started",
            "process_service",
            format!("Handling node {}", node.id()),
        );
        entered.node_id = Some(node.id().to_string());
        self.log_best_effort(process_token, entered).await;

        // Persist the stage before notifying the host application.
        if let Some(set_stage) = node.set_stage() {
            let stage_with_reason = set_stage.stage_with_reason();
            let process_def = self.get_process_def_by_instance(process_token).await?;
            let stage_def = process_def
                .stages
                .iter()
                .find(|stage| stage.id() == &set_stage.stage_with_reason().stage);
            if let Some(stage_def) = stage_def {
                self.assign_stage(process_token, stage_def).await?;
                if let Some(special_handlers) = process_def.special_handlers {
                    if let Some(on_stage_change) = special_handlers.on_stage_change {
                        self.process_handler(
                            process_token,
                            &on_stage_change.handler(),
                            Some(
                                &ContextMapBuilder::new()
                                    .set(
                                        IdField::new("stage")?,
                                        ContextValue::Object {
                                            object: json!(stage_with_reason),
                                        },
                                    )
                                    .build(),
                            ),
                            container.clone(),
                        )
                        .await?;
                    }
                }
            } else {
                error!(
                    "Stage {} not found in process definition",
                    set_stage.stage_with_reason().stage
                );
                return Err(anyhow::anyhow!(
                    "Stage {} not found in process definition",
                    set_stage.stage_with_reason().stage
                ));
            }
        }

        // Dispatch the node after its shared stage-change behavior has completed.
        match node {
            Node::Start { next, .. } => {
                let node = self
                    .db_service
                    .get_process_def_node(process_token, next)
                    .await?;
                self.queue_node(process_token, &node, None).await?;
                Ok(())
            }
            Node::End { .. } => {
                let process_def = self.get_process_def_by_instance(process_token).await?;
                if let Some(special_handlers) = process_def.special_handlers
                    && let Some(on_process_complete) = special_handlers.on_process_complete
                {
                    self.process_handler(
                        process_token,
                        &on_process_complete.handler(),
                        Some(
                            &ContextMapBuilder::new()
                                .set(
                                    IdField::new("end_node_id")?,
                                    ContextValue::String {
                                        string: node.id().to_string(),
                                    },
                                )
                                .build(),
                        ),
                        container.clone(),
                    )
                    .await?;
                }
                Ok(())
            }
            Node::ServiceTask {
                handler,
                next,
                on_error: _,
                args,
                ..
            } => {
                info!("Processing service task {}", handler);
                info!("Args: {:?}", args);
                let mut context = self.get_process_instance_context(process_token).await?;
                if let Some(handler) = container.get_handler(handler) {
                    info!("Handler found: {}", handler.get_name());
                    let mut started = ExecutionLogEvent::info(
                        "handler.started",
                        "service_task",
                        format!("Handler {} started", handler.get_name()),
                    );
                    started.node_id = Some(node.id().to_string());
                    started.handler_id = Some(handler.get_name().to_string());
                    started.details = json!({ "args": args });
                    self.log_best_effort(process_token, started).await;
                    match handler
                        .process_node(process_token, &context, args.into())
                        .await
                    {
                        Ok(HandleNodeResult { status }) => {
                            let result_name = status.name();
                            let mut completed = ExecutionLogEvent::info(
                                "handler.completed",
                                "service_task",
                                format!("Handler {} returned {result_name}", handler.get_name()),
                            );
                            completed.node_id = Some(node.id().to_string());
                            completed.handler_id = Some(handler.get_name().to_string());
                            completed.details = json!({ "result": result_name });
                            match &status {
                                HandleResultStatus::Failure => {
                                    completed.level = ExecutionLogLevel::Warning;
                                    completed.event_type = "handler.attempt_failed".into();
                                    completed.error_kind = Some("handler_failure".into());
                                    completed.error_message = Some(completed.message.clone());
                                }
                                HandleResultStatus::IllegalState(state) => {
                                    completed.level = ExecutionLogLevel::Warning;
                                    completed.event_type = "handler.attempt_illegal_state".into();
                                    completed.error_kind = Some("illegal_state".into());
                                    completed.error_message = Some(state.clone());
                                    completed.details["state"] = json!(state);
                                }
                                HandleResultStatus::Repeat(repeat_at) => {
                                    completed.level = ExecutionLogLevel::Warning;
                                    completed.event_type = "handler.repeat_requested".into();
                                    completed.details["repeat_at"] = json!(repeat_at);
                                }
                                HandleResultStatus::HandlerNotExists => {
                                    completed.level = ExecutionLogLevel::Warning;
                                    completed.event_type = "handler.reported_not_exists".into();
                                }
                                HandleResultStatus::Success(_) => {}
                            }
                            self.log_best_effort(process_token, completed).await;
                            match status {
                                HandleResultStatus::Success(context_patcher) => {
                                    context.apply_patcher(&context_patcher);
                                    self.db_service
                                        .apply_process_instance_patch(
                                            process_token,
                                            &context_patcher,
                                        )
                                        .await?;
                                    if let Some(next) = next {
                                        self.queue_next_node(process_token, &next).await?;
                                    }
                                }
                                HandleResultStatus::HandlerNotExists => {
                                    if let Some(next) = next {
                                        self.queue_next_node(process_token, &next).await?;
                                    }
                                }
                                HandleResultStatus::Failure
                                | HandleResultStatus::IllegalState(_) => {
                                    return Err(anyhow::anyhow!(
                                        "Handler {} returned {}",
                                        handler.get_name(),
                                        result_name
                                    ));
                                }
                                HandleResultStatus::Repeat(repeat_in) => {
                                    self.queue_node(process_token, node, Some(repeat_in))
                                        .await?;
                                }
                            }
                        }
                        Err(e) => {
                            let error = anyhow::Error::from(e);
                            let mut failed = ExecutionLogEvent::warning(
                                "handler.attempt_failed",
                                "service_task",
                                format!("Handler {} attempt failed: {error:#}", handler.get_name()),
                            );
                            failed.node_id = Some(node.id().to_string());
                            failed.handler_id = Some(handler.get_name().to_string());
                            failed.details["args"] = json!(args);
                            self.log_best_effort(process_token, failed).await;
                            return Err(
                                error.context(format!("Handler {} failed", handler.get_name()))
                            );
                        }
                    }
                } else {
                    let error = anyhow::anyhow!("Handler {} is not registered", handler);
                    let mut failed = ExecutionLogEvent::warning(
                        "handler.attempt_failed",
                        "service_task",
                        error.to_string(),
                    );
                    failed.message = error.to_string();
                    failed.node_id = Some(node.id().to_string());
                    failed.handler_id = Some(handler.to_string());
                    self.log_best_effort(process_token, failed).await;
                    return Err(error);
                }

                Ok(())
            }
            Node::UserTask {
                timeout,
                wait_for,
                form,
                ..
            } => {
                if let Some(timeout) = timeout {
                    let when = timeout.after_now()?;
                    self.create_event_schedule(
                        process_token,
                        node,
                        when,
                        &timeout.on_timeout,
                        wait_for.clone(),
                    )
                    .await?;
                }
                let process_def = self.get_process_def_by_instance(process_token).await?;
                if let Some(special_handlers) = process_def.special_handlers {
                    if let Some(on_show_form) = special_handlers.on_show_form {
                        if let Some(form_def) =
                            process_def.forms.iter().find(|frm| frm.id == form.clone())
                        {
                            self.process_handler(
                                process_token,
                                &on_show_form.handler(),
                                Some(
                                    &ContextMapBuilder::new()
                                        .set(
                                            IdField::new("form_id")?,
                                            ContextValue::IdField {
                                                id_field: form_def.id.clone(),
                                            },
                                        )
                                        .set(
                                            IdField::new("roles")?,
                                            ContextValue::Array {
                                                array: form_def
                                                    .roles
                                                    .iter()
                                                    .map(|r| ContextValue::IdField {
                                                        id_field: r.clone(),
                                                    })
                                                    .collect(),
                                            },
                                        )
                                        .build(),
                                ),
                                container,
                            )
                            .await?;
                        }
                    }
                }
                Ok(())
            }
            Node::Wait {
                timeout, wait_for, ..
            } => {
                if let Some(timeout) = timeout {
                    self.create_event_schedule(
                        process_token,
                        node,
                        timeout.after_now()?,
                        &timeout.on_timeout,
                        Some(wait_for.clone()),
                    )
                    .await?;
                }
                Ok(())
            }
            Node::Gateway {
                gateway,
                branches,
                next,
                ..
            } => {
                let next = self
                    .select_branch_for_process(process_token, branches, next, gateway)
                    .await?;
                let node = self.get_node(process_token, &next).await?;
                self.queue_node(process_token, &node, None).await?;
                Ok(())
            }
        }
    }

    async fn process_handler(
        &self,
        process_token: &IdField,
        handler_id: &IdField,
        args: Option<&ContextMap>,
        container: Arc<FluxproHandlersContainer>,
    ) -> anyhow::Result<HandleNodeResult> {
        let context = self.get_process_instance_context(process_token).await?;
        let handler = container.get_handler(handler_id).ok_or_else(|| {
            error!("Handler {} not found", handler_id);
            anyhow::anyhow!("Handler not found")
        })?;
        info!(
            "+++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++"
        );
        info!(
            "Invoking handler {} for process {}",
            handler_id, process_token
        );
        info!("Handler context: {:?}", context);
        info!("Handler args: {:?}", args);
        info!(
            "+++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++++"
        );
        handler
            .process_node(process_token, &context, args)
            .await
            .with_context(|| format!("Error while processing handler {}", handler_id))
    }

    async fn create_event_schedule(
        &self,
        process_token: &IdField,
        node: &Node,
        when: DateTime<Utc>,
        on_time: &IdField,
        wait_for: Option<WaitFor>,
    ) -> anyhow::Result<()> {
        let cancel_events = match wait_for {
            None => {
                vec![]
            }
            Some(WaitFor::Multi { signals }) => signals
                .into_iter()
                .map(|signal_id| {
                    let signal_id = signal_id.clone();
                    QueueTaskCancel::new(signal_id.clone(), process_token, node)
                })
                .collect(),
            Some(WaitFor::Single { signal }) => {
                vec![QueueTaskCancel::new(signal.clone(), process_token, node)]
            }
        };
        self.db_service
            .add_queue_task(
                FluxproQueueTask::ProcessEvent {
                    process_token: process_token.clone(),
                    node: node.clone(),
                    on_time: on_time.clone(),
                },
                Some(when),
                Some(cancel_events),
            )
            .await?;
        self.queue_wakeup.notify_one();
        Ok(())
    }

    async fn get_process_instance_current_node(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<Option<Node>> {
        self.db_service
            .get_process_instance_node(process_token)
            .await
    }

    async fn get_process_instance_context(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<ContextMap> {
        self.db_service
            .get_process_instance_context(process_token)
            .await
    }

    async fn eval_bool_with_context(&self, when: &str, context: &ContextMap) -> Option<bool> {
        crate::service::expressions::evaluate_condition(when, context).ok()
    }

    async fn get_process_def_by_instance(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<ProcessDefinition> {
        let process_def_db = self
            .db_service
            .get_process_def_by_instance(process_token)
            .await?
            .try_into()
            .map_err(|e| {
                error!(r#"Invalid process definition: {e}"#);
                anyhow::anyhow!("Invalid process definition")
            })?;
        Ok(process_def_db)
    }

    async fn get_process_token_by_process_id(
        &self,
        process_id: &IdField,
    ) -> anyhow::Result<IdField> {
        Ok(self
            .db_service
            .get_process_token_by_process_id(process_id)
            .await?)
    }

    async fn cancel_queue_task(
        &self,
        node_id: &IdField,
        signal: &IdField,
        process_token: &IdField,
    ) -> anyhow::Result<()> {
        self.db_service
            .cancel_queue_tasks(node_id, signal, process_token)
            .await?;
        Ok(())
    }

    async fn get_escalation_actions(
        &self,
        process_token: &IdField,
        escalation_id: &IdField,
    ) -> anyhow::Result<Vec<EscalationActionDef>> {
        self.db_service
            .get_escalation_actions(process_token, escalation_id)
            .await
    }
}

#[cfg(test)]
mod signal_delivery_tests {
    use super::*;

    fn id(value: &str) -> IdField {
        IdField::new(value).unwrap()
    }

    fn service_node() -> Node {
        Node::ServiceTask {
            id: id("send_to_erp"),
            handler: id("send_to_erp_handler"),
            args: None,
            set_stage: None,
            retries: None,
            next: None,
            on_error: None,
        }
    }

    fn waiting_node(signal: &str) -> Node {
        Node::UserTask {
            id: id("wait_documents"),
            form: id("signing_wait"),
            wait_for: Some(WaitFor::Single { signal: id(signal) }),
            set_stage: None,
            timeout: None,
            next: Some(Next::To(id("sign_documents"))),
            on_error: None,
        }
    }

    #[test]
    fn signal_is_deferred_while_a_service_node_is_finishing() {
        assert!(can_wait_for_future_signal(Some(&service_node())));
    }

    #[test]
    fn signal_is_not_deferred_after_process_end() {
        let node = Node::End {
            id: id("done"),
            set_stage: None,
        };
        assert!(!can_wait_for_future_signal(Some(&node)));
    }

    #[test]
    fn waiting_node_accepts_only_its_declared_signal() {
        let node = waiting_node("docs_arrived");

        assert!(waiting_transition(&node, &id("docs_arrived")).is_some());
        assert!(waiting_transition(&node, &id("other_signal")).is_none());
        assert!(can_wait_for_future_signal(Some(&node)));
    }

    #[test]
    fn legacy_signal_referenced_by_wait_node_is_accepted() {
        let node = waiting_node("stop_invitation");

        assert!(signal_is_declared_or_waited_for(
            &[],
            &[node],
            &id("stop_invitation")
        ));
    }

    #[test]
    fn signal_without_declaration_or_wait_node_is_rejected() {
        assert!(!signal_is_declared_or_waited_for(
            &[],
            &[],
            &id("stop_invitation")
        ));
    }

    #[test]
    fn repeated_deferral_logs_are_sampled() {
        assert!(should_log_signal_deferral(1));
        assert!(should_log_signal_deferral(50));
        assert!(!should_log_signal_deferral(49));
    }
}
