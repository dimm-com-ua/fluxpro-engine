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
use crate::service::{ctx_to_rhai_map, normalize_quotes};
use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::debug;
use log::{error, info, warn};
use rhai::{Engine, Scope};
use serde_json::json;
use sqlx::Error;
use std::sync::Arc;
use tokio::sync::Notify;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PendingTask {
    pub id: String,
    pub kind: String,
}

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

fn should_log_signal_deferral(attempts: i32) -> bool {
    attempts == 1 || attempts % 50 == 0
}

#[async_trait]
pub trait FluxproService {
    async fn create_process_def(
        &self,
        process_def: &ProcessDefinition,
    ) -> Result<ProcessId, CreateProcessError>;
    async fn create_process_def_from_source(
        &self,
        process_def: &ProcessDefinition,
        source_definition: &str,
    ) -> Result<ProcessId, CreateProcessError>;
    async fn start_process_instance(
        &self,
        process_id: IdField,
        start_process_instance: StartProcessInstance,
    ) -> Result<IdField, CreateProcessError>;
    async fn get_process(
        &self,
        process_id: IdField,
        version: Option<VersionId>,
    ) -> Result<ProcessDefinition, CreateProcessError>;
    async fn post_signal(
        &self,
        process_id: IdField,
        signal: PostSignal,
    ) -> Result<(), CreateProcessError>;
    async fn post_signal_by_process_id(
        &self,
        process_id: IdField,
        signal: PostSignal,
    ) -> Result<(), CreateProcessError>;
    async fn assign_stage(&self, process_token: &IdField, stage: &StageDef) -> Result<(), Error>;
    async fn queue_node(
        &self,
        process_token: &IdField,
        node: &Node,
        after: Option<DateTime<Utc>>,
    ) -> Result<(), Error>;
    async fn queue_next_node(&self, process_token: &IdField, next: &Next) -> anyhow::Result<()>;
    async fn queue_signal(
        &self,
        process_token: &IdField,
        signal: &PostSignal,
    ) -> anyhow::Result<()>;
    async fn select_branch_for_process(
        &self,
        process_token: &IdField,
        branches: &Vec<Branch>,
        default: &Option<IdField>,
        gateway: &GatewayKind,
    ) -> anyhow::Result<IdField>;
    async fn get_node(&self, process_token: &IdField, node_id: &IdField) -> Result<Node, Error>;
    async fn add_context_variable(
        &self,
        process_token: &IdField,
        scope: &str,
        var_id: &IdField,
        value: &ContextValue,
    ) -> Result<(), Error>;
    async fn save_process_instance_context(
        &self,
        process_token: &IdField,
        context: &ContextMap,
    ) -> anyhow::Result<()>;
    async fn fetch_queue_task(
        &self,
        lock_key: &IdField,
        lease_ms: u64,
    ) -> anyhow::Result<Option<FluxproQueueTaskDefinition>>;
    async fn get_queue_health(&self) -> anyhow::Result<FluxproQueueHealth>;
    async fn renew_queue_task_lock(
        &self,
        task_uuid: Uuid,
        lock_key: &str,
        lease_ms: u64,
    ) -> anyhow::Result<bool>;
    async fn retry_queue_task(
        &self,
        task: &FluxproQueueTaskDefinition,
        run_after: DateTime<Utc>,
    ) -> anyhow::Result<()>;
    async fn process_task(
        &self,
        task: FluxproQueueTaskDefinition,
        container: Arc<FluxproHandlersContainer>,
    ) -> anyhow::Result<TaskProcessOutcome>;
    async fn handle_node(
        &self,
        process_token: &IdField,
        node: &Node,
        container: Arc<FluxproHandlersContainer>,
    ) -> anyhow::Result<()>;
    async fn process_handler(
        &self,
        process_token: &IdField,
        handler_id: &IdField,
        args: Option<&ContextMap>,
        container: Arc<FluxproHandlersContainer>,
    ) -> anyhow::Result<HandleNodeResult>;
    async fn create_event_schedule(
        &self,
        process_token: &IdField,
        node: &Node,
        when: DateTime<Utc>,
        on_timeout: &IdField,
        cancel_events: Option<WaitFor>,
    ) -> anyhow::Result<()>;
    async fn get_process_instance_current_node(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<Option<Node>>;
    async fn get_process_instance_context(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<ContextMap>;
    async fn eval_bool_with_context(&self, when: &str, context: &ContextMap) -> Option<bool>;
    async fn get_process_def_by_instance(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<ProcessDefinition>;
    async fn get_process_token_by_process_id(
        &self,
        process_id: &IdField,
    ) -> anyhow::Result<IdField>;
    async fn cancel_queue_task(
        &self,
        node_id: &IdField,
        signal: &IdField,
        process_token: &IdField,
    ) -> anyhow::Result<()>;
    async fn get_escalation_actions(
        &self,
        process_token: &IdField,
        escalation_id: &IdField,
    ) -> anyhow::Result<Vec<EscalationActionDef>>;
    async fn record_execution_log(
        &self,
        process_token: &IdField,
        event: ExecutionLogEvent,
    ) -> anyhow::Result<()>;
}

pub struct FluxproServiceImpl {
    db_service: Arc<dyn FluxproDbService + Send + Sync>,
    queue_wakeup: Arc<Notify>,
    signal_retry_policy: SignalRetryPolicy,
}

impl FluxproServiceImpl {
    pub fn new(db_service: Arc<dyn FluxproDbService + Send + Sync>) -> Self {
        Self::with_queue_wakeup(
            db_service,
            Arc::new(Notify::new()),
            SignalRetryPolicy::default(),
        )
    }

    pub fn with_queue_wakeup(
        db_service: Arc<dyn FluxproDbService + Send + Sync>,
        queue_wakeup: Arc<Notify>,
        signal_retry_policy: SignalRetryPolicy,
    ) -> Self {
        Self {
            db_service,
            queue_wakeup,
            signal_retry_policy,
        }
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
        start_process_instance: StartProcessInstance,
    ) -> Result<IdField, CreateProcessError> {
        if let Ok(process_def) = self
            .get_process(process_id.clone(), start_process_instance.version)
            .await
        {
            if let Ok(false) = self
                .db_service
                .check_process_id(
                    process_def.uuid.unwrap(),
                    &start_process_instance.process_id,
                )
                .await
            {
                return Err(CreateProcessError::ProcessIdNonUnique);
            }

            let starting_node = process_def.nodes.iter().find(|node| node.is_start());
            let initial_stage = process_def.stages.iter().find(|stage| stage.is_initial());

            if let Some(starting_node) = starting_node
                && let Some(initial_stage) = initial_stage
            {
                match self
                    .db_service
                    .create_process_instance(
                        process_def.uuid.unwrap(),
                        start_process_instance.process_id,
                    )
                    .await
                {
                    Ok(process_token) => {
                        self.log_best_effort(
                            &process_token,
                            ExecutionLogEvent::info(
                                "instance.created",
                                "process_service",
                                "Process instance created",
                            ),
                        )
                        .await;
                        for (var_id, context_val) in start_process_instance.context.0 {
                            self.add_context_variable(&process_token, "_", &var_id, &context_val)
                                .await
                                .map_err(|e| {
                                    error!("Error while adding context variable: {}", e);
                                    e
                                })?;
                        }
                        info!("Starting node: {}", starting_node.id());
                        info!("Initial stage: {}", initial_stage.id());
                        self.assign_stage(&process_token, initial_stage)
                            .await
                            .map_err(|e| {
                                error!("Error while assigning stage: {}", e);
                                e
                            })?;
                        info!("Starting node: {}", starting_node.id());
                        self.queue_node(&process_token, starting_node, None)
                            .await
                            .map_err(|e| {
                                error!("Error while processing node: {}", e);
                                e
                            })?;
                        Ok(process_token)
                    }
                    Err(_) => Err(CreateProcessError::RunProcessError),
                }
            } else {
                error!("Starting node not found");
                Err(CreateProcessError::ProcessDefNotFound)
            }
        } else {
            Err(CreateProcessError::ProcessDefNotFound)
        }
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
                if let Ok(node) = self.get_node(process_token, to_node).await {
                    self.queue_node(process_token, &node, None).await?;
                    Ok(())
                } else {
                    Ok(())
                }
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
                if let Ok(node) = self.get_node(process_token, &to_node).await {
                    self.queue_node(process_token, &node, None).await?;
                    Ok(())
                } else {
                    Ok(())
                }
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
                    if let Some(true) = self.eval_bool_with_context(&b.when, &context).await {
                        info!("Branch evaluated to true");
                        return Ok(b.next.clone());
                    }
                }
            }
            GatewayKind::AND => {
                for b in branches {
                    info!("Evaluating branch: {}", b.when);
                    if let Some(false) = self.eval_bool_with_context(&b.when, &context).await {
                        info!("Branch evaluated to false");
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
        info!("Processing task {:?}", task);
        match &task.task {
            FluxproQueueTask::ProcessNode {
                process_token,
                node,
            } => {
                if let Ok(Some(current_node)) =
                    self.get_process_instance_current_node(process_token).await
                {
                    if let Node::UserTask { form, .. } = current_node {
                        let process_def = self.get_process_def_by_instance(process_token).await?;
                        if let Some(special_handlers) = process_def.special_handlers {
                            if let Some(on_hide_form) = special_handlers.on_hide_form {
                                self.process_handler(
                                    process_token,
                                    &on_hide_form.handler(),
                                    Some(
                                        &ContextMapBuilder::new()
                                            .set(
                                                IdField::new("form_id")?,
                                                ContextValue::IdField {
                                                    id_field: form.clone(),
                                                },
                                            )
                                            .build(),
                                    ),
                                    container.clone(),
                                )
                                .await?;
                            }
                        }
                    }
                }
                self.db_service
                    .set_process_current_node(process_token, node)
                    .await
                    .map_err(|e| {
                        error!("Error while setting current node: {}", e);
                        e
                    })?;
                let mut event = ExecutionLogEvent::info(
                    "node.entered",
                    "process_service",
                    format!("Entered node {}", node.id()),
                );
                event.node_id = Some(node.id().to_string());
                event.queue_task_uuid = Some(task.uuid);
                event.attempt = Some(task.attempts);
                event.details = json!({ "node": node });
                self.log_best_effort(process_token, event).await;
                if let Err(error) = self.handle_node(process_token, &node, container).await {
                    if let Node::ServiceTask {
                        retries, on_error, ..
                    } = &node
                    {
                        if let Some(retries) = retries
                            && task.attempts <= retries.max as i32
                        {
                            let run_after = retries.next_attempt_at()?;
                            self.retry_queue_task(&task, run_after).await?;
                            let mut event = ExecutionLogEvent::warning(
                                "queue_task.retry_scheduled",
                                "process_service",
                                format!("Task will be retried after handler failure: {error:#}"),
                            );
                            event.node_id = Some(node.id().to_string());
                            event.queue_task_uuid = Some(task.uuid);
                            event.attempt = Some(task.attempts);
                            event.details = json!({
                                "run_after": run_after,
                                "max_retries": retries.max,
                                "backoff": retries.backoff,
                            });
                            self.log_best_effort(process_token, event).await;
                            return Ok(TaskProcessOutcome::RetryScheduled);
                        }

                        let mut event = ExecutionLogEvent::error(
                            "queue_task.retries_exhausted",
                            "process_service",
                            &error,
                        );
                        event.message = format!("Task failed after {} attempt(s)", task.attempts);
                        event.node_id = Some(node.id().to_string());
                        event.queue_task_uuid = Some(task.uuid);
                        event.attempt = Some(task.attempts);
                        self.log_best_effort(process_token, event).await;
                        if let Some(next) = on_error.as_ref().and_then(|value| value.next.as_ref())
                        {
                            let error_node = self.get_node(process_token, next).await?;
                            self.queue_node(process_token, &error_node, None).await?;
                        }
                    } else {
                        return Err(error);
                    }
                }
            }
            FluxproQueueTask::ProcessEvent {
                process_token,
                node,
                on_time,
            } => {
                info!("Processing event on node {}", node.id());
                if let Ok(Some(current_node)) =
                    self.get_process_instance_current_node(process_token).await
                {
                    if current_node.id() == node.id() {
                        if let Ok(node) = self
                            .db_service
                            .get_process_def_node(process_token, &on_time)
                            .await
                        {
                            self.queue_node(process_token, &node, None).await?;
                        }
                    } else {
                        error!("Current node not equal to event node {}", node.id());
                    }
                } else {
                    error!("Current node not found");
                }
            }
            FluxproQueueTask::ProcessSignal {
                process_token,
                signal,
            } => {
                info!("Processing signal {}", signal.signal);
                let current_node = self
                    .get_process_instance_current_node(process_token)
                    .await
                    .map_err(|e| {
                        error!("Error while getting current node: {}", e);
                        e
                    })?;
                info!("Current node: {:?}", current_node);
                if let Some((current_node, next)) = current_node
                    .as_ref()
                    .and_then(|node| waiting_transition(node, &signal.signal))
                {
                    self.cancel_queue_task(current_node.id(), &signal.signal, process_token)
                        .await
                        .map_err(|e| {
                            error!("Error while canceling queue task: {}", e);
                            e
                        })?;
                    self.save_process_instance_context(process_token, &signal.context)
                        .await
                        .map_err(|e| {
                            error!("Error while saving context: {}", e);
                            e
                        })?;
                    self.add_context_variable(
                        process_token,
                        "_",
                        &IdField::new("_last_signal").unwrap(),
                        &ContextValue::String {
                            string: signal.signal.get_id().to_string(),
                        },
                    )
                    .await
                    .map_err(|e| {
                        error!("Error while adding context variable: {}", e);
                        e
                    })?;
                    if let Some(next) = next {
                        match next {
                            Next::To(next_id) => {
                                let node = self.get_node(process_token, next_id).await.map_err(|e| {
                                    error!("Error while getting a node while processing the next node: {}", e);
                                    error!("Processing next node with ID: {}", next_id);
                                    error!("Process token: {}", process_token);
                                    e
                                })?;
                                self.queue_node(process_token, &node, None)
                                    .await
                                    .map_err(|e| {
                                        error!("Error while queueing node: {}", e);
                                        e
                                    })?;
                            }
                            Next::Routes(next_routes) => {
                                let next_node = self
                                    .select_branch_for_process(
                                        process_token,
                                        &next_routes.branches,
                                        &next_routes.default,
                                        &GatewayKind::XOR,
                                    )
                                    .await
                                    .map_err(|e| {
                                        error!("Error while selecting branch for process: {}", e);
                                        e
                                    })?;
                                let node = self.get_node(process_token, &next_node).await.map_err(
                                    |e| {
                                        error!(
                                            "Error while getting node after selecting branch: {}",
                                            e
                                        );
                                        e
                                    },
                                )?;
                                self.queue_node(process_token, &node, None)
                                    .await
                                    .map_err(|e| {
                                        error!("Error while queueing node: {}", e);
                                        e
                                    })?;
                            }
                        }
                    }
                    let mut event = ExecutionLogEvent::info(
                        "signal.accepted",
                        "process_service",
                        format!("Signal {} was accepted", signal.signal),
                    );
                    event.node_id = Some(current_node.id().to_string());
                    event.queue_task_uuid = Some(task.uuid);
                    event.attempt = Some(task.attempts);
                    event.details = json!({
                        "signal": signal.signal,
                        "queued_at": task.created_at,
                        "delivery_time_ms": Utc::now()
                            .signed_duration_since(task.created_at)
                            .num_milliseconds()
                            .max(0),
                    });
                    self.log_best_effort(process_token, event).await;
                } else if can_wait_for_future_signal(current_node.as_ref())
                    && let Some(retry) = self
                        .signal_retry_policy
                        .decision(task.created_at, Utc::now())
                {
                    self.retry_queue_task(&task, retry.retry_at).await?;

                    if should_log_signal_deferral(task.attempts) {
                        let mut event = ExecutionLogEvent::warning(
                            "signal.deferred",
                            "process_service",
                            format!(
                                "Signal {} is waiting for a compatible process node",
                                signal.signal
                            ),
                        );
                        event.queue_task_uuid = Some(task.uuid);
                        event.attempt = Some(task.attempts);
                        event.details = json!({
                            "signal": signal.signal,
                            "current_node": current_node.as_ref().map(|node| node.id()),
                            "retry_phase": retry.phase,
                            "retry_at": retry.retry_at,
                            "retry_delay_ms": retry.delay.num_milliseconds(),
                            "age_ms": retry.age.num_milliseconds(),
                            "expires_at": retry.expires_at,
                        });
                        self.log_best_effort(process_token, event).await;
                    }

                    return Ok(TaskProcessOutcome::RetryScheduled);
                } else if can_wait_for_future_signal(current_node.as_ref()) {
                    let mut event = ExecutionLogEvent::critical(
                        "signal.expired",
                        "process_service",
                        format!(
                            "Signal {} was not accepted before its delivery deadline",
                            signal.signal
                        ),
                    );
                    event.queue_task_uuid = Some(task.uuid);
                    event.attempt = Some(task.attempts);
                    event.details = json!({
                        "signal": signal.signal,
                        "current_node": current_node.as_ref().map(|node| node.id()),
                        "queued_at": task.created_at,
                        "expired_at": Utc::now(),
                        "delivery_horizon_ms": self.signal_retry_policy
                            .expires_after()
                            .num_milliseconds(),
                    });
                    self.log_best_effort(process_token, event).await;
                } else {
                    let mut event = ExecutionLogEvent::warning(
                        "signal.not_accepted",
                        "process_service",
                        format!(
                            "Signal {} is not accepted by the current process node",
                            signal.signal
                        ),
                    );
                    event.queue_task_uuid = Some(task.uuid);
                    event.attempt = Some(task.attempts);
                    event.details = json!({
                        "signal": signal.signal,
                        "current_node": current_node.as_ref().map(|node| node.id()),
                    });
                    self.log_best_effort(process_token, event).await;
                }
            }
        }
        self.db_service.kill_queue_task(&task).await.map_err(|e| {
            error!("Error while killing queue task: {}", e);
            e
        })?;
        Ok(TaskProcessOutcome::Completed)
    }

    /// Handles the execution flow for a single workflow **node** belonging to a process instance.
    ///
    /// This method is the core dispatcher that:
    ///
    /// 1. **Detects state changes** on the current node (e.g. a user‑task form being hidden or a stage change) and invokes any
    ///    *special handlers* defined in the process definition (such as `on_hide_form`,
    ///    `on_stage_change`, `on_show_form`, etc.).
    /// 2. **Routes** the node based on its concrete variant (`Start`, `End`, `ServiceTask`,
    ///    `UserTask`, `Gateway`) and performs the appropriate actions:
    ///    - `Start`     → fetches the next node and queues it for execution.
    ///    - `End`      → terminates the handling path (nothing further to do).
    ///    - `ServiceTask` → runs the associated handler, processes its result status
    ///      (`Success`, `Failure`, `Repeat`, …), updates the process context, and queues the
    ///      subsequent node if applicable.
    ///    - `UserTask`  → schedules a timeout event (if configured) and triggers the
    ///      `on_show_form` special handler with form metadata.
    ///    - `Gateway`   → selects a branch according to the gateway logic and queues the
    ///      chosen next node.
    ///
    /// All interactions with the persistence layer (`db_service`) and the handler container
    /// (`FluxproHandlersContainer`) are performed asynchronously, and any encountered error
    /// propagates as an `anyhow::Error`.
    ///
    /// # Arguments
    ///
    /// * `process_token` – Identifier of the process instance whose node is being handled.
    /// * `node` – Reference to the `Node` that should be processed.  The node type drives
    ///   the control flow described above.
    /// * `container` – Shared (`Arc`) container holding all registered handlers.  It is
    ///   cloned where needed so that the async calls can own a reference.
    ///
    /// # Returns
    ///
    /// * `Ok(())` – The node was processed successfully (including any side‑effects such
    ///   as queuing subsequent nodes, updating context, or invoking special handlers).
    /// * `Err(anyhow::Error)` – Propagation of any failure from:
    ///   - Retrieving the current node or process definition.
    ///   - Looking up or executing a handler.
    ///   - Persisting context or queue entries.
    ///   - Scheduling events or evaluating gateway logic.
    ///
    /// # Errors
    ///
    /// The method bubbles up errors from the underlying services. Typical failure
    /// scenarios include:
    ///
    /// * Missing handler IDs (`handler()` resolves to an unknown handler).
    /// * Database errors when fetching nodes, saving context, or inserting queue tasks.
    /// * Serialization problems when building `ContextMap`s for special handlers.
    /// * Unexpected `None` values when required process definitions or forms are absent.
    ///
    /// # Remarks
    ///
    /// * **Special Handlers** – Before the main node‑type dispatch, the method checks
    ///   whether the *current* node (as stored in the DB) is a `UserTask` and, if so,
    ///   runs `on_hide_form`.  Later, when handling the incoming `node`, it may run
    ///   `on_show_form` (for `UserTask`) or `on_stage_change` (if the node defines a
    ///   stage transition).  These hooks allow custom business logic to be injected
    ///   without hard‑coding it into the core engine.
    ///
    /// * **Context Management** – For `ServiceTask` handling, the process instance
    ///   context is fetched, passed to the handler, and then patched back into the
    ///   stored context only on successful completion (`HandleResultStatus::Success`).
    ///
    /// * **Async Cloning** – The `container` is an `Arc`; cloning it is cheap and enables
    ///   the handler calls to own a reference across `.await` points without borrowing
    ///   the original `self`.
    ///
    /// * **Idempotency** – The method assumes that each node is queued exactly once.
    ///   Re‑queuing (e.g., via `HandleResultStatus::Repeat`) is performed by
    ///   re‑invoking `queue_node` with the original `node` and a delay.
    ///
    /// # Example (simplified)
    ///
    /// ```rust,ignore
    /// // Assume `service` implements the trait containing `handle_node`.
    /// let proc_id = IdField::new("process-123")?;
    /// let node = service.get_node(&proc_id, "node-abc").await?;
    /// let handlers = Arc::new(FluxproHandlersContainer::new());
    ///
    /// // Process the node – any errors bubble up as `anyhow::Error`.
    /// service.handle_node(&proc_id, &node, handlers).await?;
    /// ```
    ///
    /// This call will transparently execute any configured special handlers,
    /// schedule timeouts, evaluate gateways, and advance the workflow to the next
    /// node according to the process definition.
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

        // process set_stage handler if presents
        //
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

        // process handler accordion to the node type
        //
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
                                    self.save_process_instance_context(process_token, &context)
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

    /// Executes a registered handler for a given process.
    ///
    /// This method:
    ///   1. Retrieves the context associated with `process_token`.
    ///   2. Looks up the handler identified by `handler_id` in the supplied
    ///      `FluxproHandlersContainer`.
    ///   3. If the handler exists, forwards the call to its `process_node`
    ///      implementation, passing the process token, the fetched context,
    ///      and any optional arguments.
    ///
    /// # Arguments
    ///
    /// * `process_token` – Identifier of the process instance whose context
    ///   should be loaded.
    /// * `handler_id` – Identifier of the handler to invoke. The handler must
    ///   be pre‑registered in `container`.
    /// * `args` – Optional additional arguments supplied to the handler. If
    ///   `None`, the handler receives no extra data.
    /// * `container` – Shared reference‑counted container that holds all
    ///   registered handlers. The container is cloned as an `Arc` so it can be
    ///   safely used across `await` points.
    ///
    /// # Returns
    ///
    /// * `Ok(HandleNodeResult)` – The result produced by the handler’s
    ///   `process_node` call.
    /// * `Err(anyhow::Error)` – Either
    ///   - The process context could not be retrieved, or
    ///   - No handler matching `handler_id` was found, or
    ///   - The handler itself returned an error.
    ///
    /// # Errors
    ///
    /// The error message includes the offending identifier, e.g.
    /// `"Handler <handler_id> not found"` to aid debugging.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let result = my_service
    ///     .process_handler(&proc_id, &handler_id, None, handlers.clone())
    ///     .await?;
    /// // `result` is a `HandleNodeResult` from the invoked handler
    /// ```
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

    /// Schedules a process‑event queue task to be executed at a specific point in time.
    ///
    /// This method builds a `FluxproQueueTask::ProcessEvent` entry and registers it
    /// with the underlying database service.  Optionally it also creates cancellation
    /// tasks that will be triggered if the scheduled event needs to be aborted
    /// (e.g. when waiting on one or more signals).
    ///
    /// # Arguments
    ///
    /// * `process_token` – Identifier of the process instance that owns the event.
    /// * `node` – The workflow node that should be processed when the event fires.
    /// * `when` – UTC timestamp indicating when the event should be queued for
    ///   execution.  The timestamp is passed straight through to the DB layer, so
    ///   it must be a concrete `date_time<Utc>` value.
    /// * `on_time` – Identifier of the *on‑time* signal that will be emitted once the
    ///   scheduled moment arrives.  This ID is stored in the queue task payload.
    /// * `wait_for` – Optional condition that determines which cancellation signals
    ///   should be attached to the task:
    ///   - `None` – No cancellation signals are generated.
    ///   - `Some(WaitFor::Single { signal })` – A single cancellation task is
    ///     created for the provided `signal`.
    ///
    ///   - `Some(WaitFor::Multi { signals })` – One cancellation task is created for
    ///     each `signal` in the supplied collection.
    ///
    ///
    /// # Returns
    ///
    /// * `Ok(())` – The schedule was successfully persisted in the database.
    /// * `Err(anyhow::Error)` – Propagates any error returned by the underlying
    ///   `db_service.add_queue_task` call (e.g., DB connectivity issues, constraint
    ///   violations, serialization problems).
    ///
    ///
    /// # Errors
    ///
    /// The error will contain context from the database layer.  Typical failure
    /// modes include:
    ///   - Inability to serialize the `FluxproQueueTask` payload.
    ///   - Database transaction aborts.
    ///
    ///
    /// # Example
    /// ```rust,ignore
    /// let when = Utc::now() + chrono::Duration::minutes(5);
    /// my_service
    ///    .create_event_schedule(
    ///        &process_id,
    ///        &node,
    ///        when,
    ///        &on_time_signal,
    ///        Some(WaitFor::Single { signal: cancel_signal }),
    ///    )
    ///    .await?;
    /// // The event will fire in five minutes, unless `cancel_signal` is raised.
    /// ```
    /// # Remarks
    ///
    /// * The method is `async` because persisting the queue task involves I/O with the
    ///   database service.
    /// * `cancel_events` is built eagerly before the DB call so that the cancellation
    ///   logic is captured atomically with the scheduled task.
    /// * `id_field` is cloned only where necessary; the rest of the parameters are
    ///   passed by reference to avoid unnecessary allocations.
    /// ```
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

    /// Retrieves the **current node** of a running process instance.
    ///
    /// The method delegates the lookup to the underlying `db_service`, which
    /// queries the persistence layer for the node that is presently active for
    /// the given process token.  If the process has not yet been started, or if
    /// it has already completed, the database may return `NULL`; in that case
    /// this function yields `Ok(None)`.
    ///
    /// # Arguments
    ///
    /// * `process_token` – The unique identifier (`id_field`) of the process
    ///   instance whose active node you want to inspect.
    ///
    /// # Returns
    ///
    /// * `Ok(Some(node))` – The process is active and the database returned the
    ///   corresponding `Node` record.\n
    /// * `Ok(None)` – No current node exists (e.g. the process is not started or
    ///   has finished).\n
    /// * `Err(anyhow::Error)` – An error occurred while communicating with the
    ///   database (connection failure, query error, deserialization problem, etc.).
    ///
    /// # Errors
    ///
    /// Any error from `db_service.get_process_instance_node` is propagated
    /// unchanged, wrapped in `anyhow::Error`.  Typical failure reasons include:
    ///
    /// * Database connectivity loss.
    /// * Corrupted or missing data for the supplied `process_token`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Assume `service` implements the trait containing this method
    /// let maybe_node = service
    ///     .get_process_instance_current_node(&process_id)
    ///     .await?;
    ///
    /// match maybe_node {
    ///     Some(node) => println!("Current node: {:?}", node),
    ///     None => println!("Process has no active node."),
    /// }
    /// ```
    ///
    /// # Remarks
    ///
    /// * The function is `async` because the underlying DB call performs I/O.
    /// * It returns an `Option<Node>` rather than panicking when the process does
    ///   not have a current node, allowing callers to handle the “no‑node” case
    ///   gracefully.
    /// * No additional transformation is performed; the raw result from the DB
    ///   service is forwarded directly to the caller.
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

    /// Evaluates a boolean expression against a runtime context.
    ///
    /// This helper builds a **Rhai** scripting engine, injects the supplied
    /// `ContextMap` under the variable name `ctx`, and then evaluates the string
    /// `when` as a boolean expression.  The function is deliberately defensive:
    /// if the script fails to compile or run, it returns `None` instead of
    /// propagating an error.
    ///
    /// # Arguments
    ///
    /// * `when` – A string containing a Rhai expression that should resolve to a
    ///   boolean value.  The expression may reference the `ctx` variable (the
    ///   converted `ContextMap`).  Example:
    ///   ```text
    ///   ctx["status"] == "active" && ctx["attempts"] > 3
    ///   ```
    /// * `context` – A map of key/value pairs (`ContextMap`) that provides the
    ///   data visible to the expression.  The map is transformed into a Rhai
    ///   `Map` via `ctx_to_rhai_map` and bound to the script as `ctx`.
    ///
    /// # Returns
    ///
    /// * `Some(true)`  – The expression compiled and evaluated successfully,
    ///   yielding `true`.
    /// * `Some(false)` – The expression compiled and evaluated successfully,
    ///   yielding `false`.
    /// * `None`        – The expression could not be parsed, exceeded the engine
    ///   limits, or resulted in a runtime error (e.g., type mismatch).  The
    ///   caller can treat this as “evaluation failed”.
    ///
    /// # Behaviour Details
    ///
    /// * **Engine configuration** – The engine is instantiated fresh on each call
    ///   with the following safety limits:
    ///   - `max_operations = 50_000` – caps total operations to prevent runaway
    ///     scripts.
    ///   - `max_expr_depths = (64, 32)` – limits nesting depth for expressions
    ///     and statements respectively.
    /// * **Logging** – The engine’s `print` and `debug` callbacks forward messages
    ///   to the crate’s `log` macros (`info!` and `debug!`).  This makes it easy
    ///   to trace evaluation steps when the log level is set appropriately.
    /// * **Quote normalization** – The input string is passed through
    ///   `normalize_quotes` before evaluation.  This utility typically converts
    ///   typographic quotes (e.g., “ ”) to plain ASCII quotes so the parser can
    ///   understand them.
    /// * **Scope preparation** – A new `Scope` is created for each evaluation;
    ///   the converted context map is inserted as a variable named `ctx`.  No
    ///   other variables are exposed, keeping the sandbox tight.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Suppose `ContextMap` is a simple HashMap<String, Value>.
    /// let mut ctx = ContextMap::new();
    /// ctx.insert("status".into(), "active".into());
    /// ctx.insert("attempts".into(), 5.into());
    ///
    /// let expr = r#"ctx["status"] == "active" && ctx["attempts"] > 3"#;
    ///
    /// let result = my_service.eval_bool_with_context(expr, &ctx).await;
    ///
    /// assert_eq!(result, Some(true));
    /// ```
    ///
    /// # Remarks
    ///
    /// * The function is `async` only because the surrounding trait/interface
    ///   expects async methods; the body itself performs no asynchronous work.
    /// * Returning `Option<bool>` instead of `Result<bool, Error>` keeps the API
    ///   simple for callers that merely need a truthy/falsey answer and can ignore
    ///   why evaluation failed.  If you need richer error information, consider
    ///   exposing a variant that returns `Result<bool, rhai::EvalAltResult>`.
    async fn eval_bool_with_context(&self, when: &str, context: &ContextMap) -> Option<bool> {
        let mut engine = Engine::new();
        engine.set_max_operations(50_000);
        engine.set_max_expr_depths(64, 32);
        engine.on_print(|print| {
            info!("{}", print);
        });
        engine.on_debug(move |s, src, pos| {
            debug!(
                "{}",
                format!("{} @ {:?} > {}", src.unwrap_or("unknown"), pos, s)
            )
        });

        let expr = normalize_quotes(when);

        let mut scope = Scope::new();
        let ctx_map = ctx_to_rhai_map(context);
        scope.push("ctx", ctx_map);

        info!("Evaluating expression: {}", expr);
        match engine.eval_with_scope::<bool>(&mut scope, &expr) {
            Ok(b) => {
                info!("Expression evaluated to: {}", b);
                Some(b)
            }
            Err(_) => None,
        }
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
