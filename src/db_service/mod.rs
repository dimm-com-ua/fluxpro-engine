//! PostgreSQL persistence, queue leases, and transactional signal admission.

use crate::db_models::process_def_db::ProcessDefDb;
use crate::db_models::process_instance_context_db::ProcessInstanceContextVariableDb;
use crate::models::commands::post_signal::PostSignal;
use crate::models::context_map::context_map::{ContextMap, ContextValue};
use crate::models::execution_log::ExecutionLogEvent;
use crate::models::id_field::IdField;
use crate::models::process_def::escalation_def::{EscalationActionDef, EscalationDef};
use crate::models::process_def::{Node, ProcessDefinition, StageDef};
use crate::models::queue::queue_task::{FluxproQueueTask, FluxproQueueTaskDefinition};
use crate::models::queue::queue_task_cancel::QueueTaskCancel;
use crate::models::version_id::VersionId;
use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::{error, info};
use serde_json::json;
use sqlx::types::JsonValue;
use sqlx::{Error, Pool, Postgres, Row};
use std::collections::HashMap;
use uuid::Uuid;

pub mod incidents;
pub mod task_execution;
use crate::models::commands::start_process_instance::StartProcessInstance;
use task_execution::{TaskExecutionChanges, TaskExecutionSnapshot};

/// Queue counts and oldest-ready timestamp for runner health reporting.
#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct FluxproQueueHealth {
    /// Number of due tasks with no live lease.
    pub ready_tasks: i64,
    /// Number of tasks with live leases.
    pub locked_tasks: i64,
    /// Earliest scheduled time among ready tasks, if any.
    pub oldest_ready_at: Option<DateTime<Utc>>,
}

/// Persistence contract for definitions, instances, context, and queue operations.
#[async_trait]
pub trait FluxproDbService {
    /// Reads the unresolved incident for an instance.
    async fn get_open_incident(
        &self,
        _token: &IdField,
    ) -> anyhow::Result<Option<incidents::ProcessIncident>> {
        anyhow::bail!("database adapter must implement incident inspection")
    }
    /// Resumes only the specified unresolved incident, retaining the task identity.
    async fn resume_instance(&self, _token: &IdField, _incident: Uuid) -> anyhow::Result<bool> {
        anyhow::bail!("database adapter must implement incident recovery")
    }
    /// Applies ordered default-scope mutations atomically and advances the instance revision.
    async fn apply_process_instance_patch(
        &self,
        _token: &IdField,
        _patch: &crate::models::context_map::context_patcher::ContextPatcher,
    ) -> anyhow::Result<()> {
        anyhow::bail!("database adapter must implement atomic context patches")
    }

    /// Reads a consistent task snapshot and verifies current lease ownership.
    ///
    /// Custom adapters must implement this contract; the default fails closed.
    async fn load_task_execution(
        &self,
        _task: &FluxproQueueTaskDefinition,
    ) -> anyhow::Result<TaskExecutionSnapshot> {
        anyhow::bail!("atomic task execution is not supported by this database adapter")
    }
    /// Commits all task effects, wait completion, and source-task disposition atomically.
    ///
    /// Must reject stale instance revisions and lost leases without partial writes.
    async fn commit_task_execution(
        &self,
        _task: &FluxproQueueTaskDefinition,
        _snapshot: &TaskExecutionSnapshot,
        _changes: TaskExecutionChanges,
    ) -> anyhow::Result<()> {
        anyhow::bail!("atomic task execution is not supported by this database adapter")
    }
    /// Creates the instance, initial context and stage, and Start task in one transaction.
    async fn start_process_instance_atomic(
        &self,
        _definition: &ProcessDefinition,
        _command: StartProcessInstance,
    ) -> anyhow::Result<IdField> {
        anyhow::bail!("atomic instance startup is not supported by this database adapter")
    }

    /// Stores a compiled definition and its declarations in one transaction.
    async fn create_service_process_def(
        &self,
        process_def: &ProcessDefinition,
        compiled_process_def: JsonValue,
        source_definition: Option<&str>,
    ) -> Result<Uuid, sqlx::Error>;
    /// Publishes a fresh active version, atomically checking the base and version order.
    /// `None` creates a new process key; `Some` requires an existing version of that key.
    async fn publish_service_process_def(
        &self,
        _process_def: &ProcessDefinition,
        _compiled_process_def: JsonValue,
        _source_definition: &str,
        _base_uuid: Option<Uuid>,
    ) -> Result<Uuid, sqlx::Error> {
        Err(sqlx::Error::Protocol(
            "Atomic definition publication is not supported by this adapter".into(),
        ))
    }
    /// Loads the latest inserted definition matching the key and effective dates.
    ///
    /// An explicit version narrows the lookup; only active versions are eligible.
    async fn get_current_process_def(
        &self,
        process_def_id: IdField,
        version: Option<VersionId>,
    ) -> Result<ProcessDefDb, sqlx::Error>;
    /// Loads the exact definition version bound to an instance token.
    async fn get_process_def_by_instance(
        &self,
        process_token: &IdField,
    ) -> Result<ProcessDefDb, sqlx::Error>;
    /// Persists a business instance and returns a generated runtime token.
    async fn create_process_instance(
        &self,
        process_def_uuid: Uuid,
        process_id: IdField,
    ) -> Result<IdField, sqlx::Error>;
    /// Returns whether the business ID is unused for the supplied definition.
    ///
    /// The database additionally enforces business-ID uniqueness across definitions.
    async fn check_process_id(
        &self,
        process_def_uuid: Uuid,
        process_id: &IdField,
    ) -> Result<bool, sqlx::Error>;
    /// Resolves a stage ID within the instance's definition.
    async fn get_stage_uuid(
        &self,
        process_token: &IdField,
        stage_id: &IdField,
    ) -> Result<Uuid, sqlx::Error>;

    /// Updates the instance stage and records history using the supplied reason.
    async fn set_process_instance_stage(
        &self,
        process_def_uuid: &IdField,
        stage_uuid: Uuid,
        reason: Option<String>,
    ) -> Result<(), sqlx::Error>;
    /// Stores a typed context value under the supplied scope and key.
    async fn add_context_variable(
        &self,
        process_token: &IdField,
        scope: &str,
        var_id: &IdField,
        value: &ContextValue,
    ) -> Result<(), sqlx::Error>;
    /// Loads a node definition by its ID and owning instance token.
    async fn get_process_def_node(
        &self,
        process_token: &IdField,
        node_id: &IdField,
    ) -> Result<Node, sqlx::Error>;
    /// Resolves a node ID to its database row identity for this instance.
    async fn get_process_def_node_uuid(
        &self,
        process_token: &IdField,
        node_id: &IdField,
    ) -> Result<Uuid, sqlx::Error>;
    /// Updates the instance's current-node reference.
    async fn set_process_current_node(
        &self,
        process_token: &IdField,
        node: &Node,
    ) -> Result<(), sqlx::Error>;
    /// Stores a queue payload and optional signal cancellation associations atomically.
    async fn add_queue_task<'a>(
        &self,
        queue_task: FluxproQueueTask,
        run_after: Option<DateTime<Utc>>,
        cancel_events: Option<Vec<QueueTaskCancel<'a>>>,
    ) -> Result<(), sqlx::Error>;
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
    /// Deletes an owned queue item and its cancellation associations.
    async fn kill_queue_task(&self, task: &FluxproQueueTaskDefinition) -> anyhow::Result<()>;
    /// Inserts a legacy `event_schedule` row; the queue runner does not consume it.
    ///
    /// Runtime timeouts use the workflow service's queued event schedule instead.
    async fn create_event_schedule(
        &self,
        process_token: &IdField,
        node: &Node,
        when: DateTime<Utc>,
        on_time: &IdField,
    ) -> anyhow::Result<()>;
    /// Loads the current node, returning `None` before initial entry.
    ///
    /// Missing instances, read errors, and malformed node JSON return errors.
    async fn get_process_instance_node(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<Option<Node>>;
    /// Loads typed variables into one map keyed by name; scopes are not preserved.
    async fn get_process_instance_context(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<ContextMap>;
    /// Merges supplied context keys into the default `_` scope.
    ///
    /// Keys absent from the supplied map remain stored; this is not a full replacement.
    async fn save_process_instance_context(
        &self,
        process_token: &IdField,
        context: &ContextMap,
    ) -> anyhow::Result<()>;
    /// Removes queued events associated with a node, signal, and instance token.
    async fn cancel_queue_tasks(
        &self,
        node_id: &IdField,
        signal_id: &IdField,
        process_token: &IdField,
    ) -> anyhow::Result<()>;
    /// Resolves a business process ID to its runtime instance token.
    async fn get_process_token_by_process_id(
        &self,
        process_id: &IdField,
    ) -> anyhow::Result<IdField>;
    /// Loads the declared actions for a topic in the instance's definition.
    async fn get_escalation_actions(
        &self,
        process_token: &IdField,
        escalation_topic: &IdField,
    ) -> anyhow::Result<Vec<EscalationActionDef>>;
    /// Records signal admission history without enqueueing delivery.
    async fn log_signal(&self, process_token: &IdField, signal: &PostSignal) -> anyhow::Result<()>;
    /// Returns the most recently admitted signal for inspection.
    async fn get_last_signal(&self, process_token: &IdField) -> anyhow::Result<Option<PostSignal>>;
    /// Persists a signal and enqueues its delivery as one operation.
    ///
    /// Adapters must persist event identities and queue delivery atomically.
    /// The default fails closed until the adapter implements this protocol.
    async fn enqueue_signal_if_new(
        &self,
        process_token: &IdField,
        signal: &PostSignal,
    ) -> anyhow::Result<bool> {
        let _ = (process_token, signal);
        anyhow::bail!("database adapter must implement atomic event identity admission")
    }

    /// Inserts a structured event into the instance's execution history.
    async fn write_execution_log(
        &self,
        process_token: &IdField,
        event: &ExecutionLogEvent,
    ) -> anyhow::Result<()>;
}

/// SQLx implementation using explicitly qualified `fluxpro` tables.
pub struct FluxproDbServiceImpl {
    db_pool: Pool<Postgres>,
}

impl FluxproDbServiceImpl {
    /// Uses an existing PostgreSQL pool without applying migrations.
    pub fn new(db_pool: Pool<Postgres>) -> Self {
        Self { db_pool }
    }
}

#[async_trait]
impl FluxproDbService for FluxproDbServiceImpl {
    async fn get_open_incident(
        &self,
        token: &IdField,
    ) -> anyhow::Result<Option<incidents::ProcessIncident>> {
        FluxproDbServiceImpl::get_open_incident(self, token).await
    }
    async fn resume_instance(&self, token: &IdField, incident: Uuid) -> anyhow::Result<bool> {
        FluxproDbServiceImpl::resume_instance(self, token, incident).await
    }
    async fn apply_process_instance_patch(
        &self,
        token: &IdField,
        patch: &crate::models::context_map::context_patcher::ContextPatcher,
    ) -> anyhow::Result<()> {
        let mut tx = self.db_pool.begin().await?;
        let instance: Uuid = sqlx::query_scalar(
            "update fluxpro.process_instance set revision=revision+1 where token=$1 returning uuid",
        )
        .bind(token.get_id())
        .fetch_one(&mut *tx)
        .await?;
        task_execution::apply_patch(&mut tx, instance, patch).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn load_task_execution(
        &self,
        task: &FluxproQueueTaskDefinition,
    ) -> anyhow::Result<TaskExecutionSnapshot> {
        task_execution::load(self, task).await
    }
    async fn commit_task_execution(
        &self,
        task: &FluxproQueueTaskDefinition,
        snapshot: &TaskExecutionSnapshot,
        changes: TaskExecutionChanges,
    ) -> anyhow::Result<()> {
        task_execution::commit(self, task, snapshot, changes).await
    }
    async fn start_process_instance_atomic(
        &self,
        definition: &ProcessDefinition,
        command: StartProcessInstance,
    ) -> anyhow::Result<IdField> {
        task_execution::start(self, definition, command).await
    }

    async fn write_execution_log(
        &self,
        process_token: &IdField,
        event: &ExecutionLogEvent,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"insert into fluxpro.process_instance_log
               (process_instance_uuid, level, event_type, source, message, node_id,
                handler_id, queue_task_uuid, attempt, error_kind, error_message, details)
               select uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12
                 from fluxpro.process_instance where token = $1"#,
        )
        .bind(process_token.get_id())
        .bind(event.level.to_string())
        .bind(&event.event_type)
        .bind(&event.source)
        .bind(&event.message)
        .bind(&event.node_id)
        .bind(&event.handler_id)
        .bind(event.queue_task_uuid)
        .bind(event.attempt)
        .bind(&event.error_kind)
        .bind(&event.error_message)
        .bind(&event.details)
        .execute(&self.db_pool)
        .await?;
        Ok(())
    }

    async fn create_service_process_def(
        &self,
        process_def: &ProcessDefinition,
        compiled_process_def: JsonValue,
        source_definition: Option<&str>,
    ) -> Result<Uuid, sqlx::Error> {
        self.register_process_definition(
            process_def,
            compiled_process_def,
            source_definition,
            false,
            None,
        )
        .await
    }

    async fn publish_service_process_def(
        &self,
        process_def: &ProcessDefinition,
        compiled_process_def: JsonValue,
        source_definition: &str,
        base_uuid: Option<Uuid>,
    ) -> Result<Uuid, sqlx::Error> {
        self.register_process_definition(
            process_def,
            compiled_process_def,
            Some(source_definition),
            true,
            base_uuid,
        )
        .await
    }

    async fn get_current_process_def(
        &self,
        process_def_id: IdField,
        version: Option<VersionId>,
    ) -> Result<ProcessDefDb, sqlx::Error> {
        let version_id = version.map(|version| version.as_str().to_string());
        let process_def_db = sqlx::query_as(
            r#"
                        SELECT *
                        FROM fluxpro.process_definition
                        WHERE key_ = $1
                          AND lower(status) = 'active'
                          AND ( $2::text IS NULL OR version = $2 )
                          AND effective_from <= $3
                          AND (deprecated_at IS NULL OR deprecated_at > $3)
                        ORDER BY index_id DESC
                        LIMIT 1
            "#,
        )
        .bind(process_def_id.get_id())
        .bind(version_id)
        .bind(Utc::now())
        .fetch_one(&self.db_pool)
        .await?;
        Ok(process_def_db)
    }

    async fn get_process_def_by_instance(
        &self,
        process_token: &IdField,
    ) -> Result<ProcessDefDb, Error> {
        let process_def_db = sqlx::query_as(
            r#"
                        SELECT *
                        FROM fluxpro.process_definition
                        WHERE uuid = (select process_def_uuid from fluxpro.process_instance where token = $1)
            "#).bind(process_token.get_id())
            .fetch_one(&self.db_pool)
            .await?;
        Ok(process_def_db)
    }

    async fn create_process_instance(
        &self,
        process_def_uuid: Uuid,
        process_id: IdField,
    ) -> Result<IdField, Error> {
        let token = IdField::generate();
        sqlx::query(
            r#"insert into fluxpro.process_instance (token, process_def_uuid, process_id) values ($1, $2, $3)"#)
            .bind(token.get_id())
            .bind(process_def_uuid)
            .bind(process_id.get_id())
            .execute(&self.db_pool)
            .await?;
        Ok(token)
    }

    async fn check_process_id(
        &self,
        process_def_uuid: Uuid,
        process_id: &IdField,
    ) -> Result<bool, Error> {
        let count: i32 = sqlx::query_scalar(
            r#"select count(uuid)::int from fluxpro.process_instance where
                    process_def_uuid=$1 and process_id=$2"#,
        )
        .bind(process_def_uuid)
        .bind(process_id.get_id())
        .fetch_one(&self.db_pool)
        .await?;
        Ok(count < 1)
    }

    async fn get_stage_uuid(
        &self,
        process_token: &IdField,
        stage_id: &IdField,
    ) -> Result<Uuid, Error> {
        Ok(sqlx::query_scalar(
            r#"select uuid from fluxpro.process_stage where process_def_uuid=
                    (select process_def_uuid from fluxpro.process_instance where token=$1) and stage_id=$2"#
        ).bind(process_token.get_id())
            .bind(stage_id.get_id())
            .fetch_one(&self.db_pool)
            .await?
        )
    }

    async fn set_process_instance_stage(
        &self,
        process_token: &IdField,
        stage_uuid: Uuid,
        reason: Option<String>,
    ) -> Result<(), sqlx::Error> {
        let mut tr = self.db_pool.begin().await?;
        sqlx::query("update fluxpro.process_instance set revision=revision+1 where token=$1")
            .bind(process_token.get_id())
            .execute(&mut *tr)
            .await?;
        sqlx::query(
            r#"insert into fluxpro.process_instance_stage_log (process_instance_uuid, stage_uuid, reason) VALUES
                               ((select uuid from fluxpro.process_instance where token = $1), $2, $3)"#
        ).bind(process_token.get_id())
            .bind(stage_uuid)
            .bind(&reason)
            .execute(&mut *tr)
            .await?;
        sqlx::query(
            r#"update fluxpro.process_instance set current_stage=$1, current_stage_reason=$2 where token=$3"#
        ).bind(stage_uuid)
            .bind(reason)
            .bind(process_token.get_id())
            .execute(&mut *tr)
            .await?;
        let _ = tr.commit().await?;
        Ok(())
    }

    async fn add_context_variable(
        &self,
        process_token: &IdField,
        scope: &str,
        var_id: &IdField,
        value: &ContextValue,
    ) -> Result<(), Error> {
        info!(
            "[add_context_variable] scope: {}, var_id: {}, value: {:?}",
            scope, var_id, value
        );
        let mut tr = self.db_pool.begin().await?;
        sqlx::query("update fluxpro.process_instance set revision=revision+1 where token=$1")
            .bind(process_token.get_id())
            .execute(&mut *tr)
            .await?;
        sqlx::query(
            r#"delete from fluxpro.process_instance_context_variable
                    where process_instance_uuid=(select uuid from fluxpro.process_instance where token=$1) and scope=$2 and name=$3"#
        ).bind(process_token.get_id())
            .bind(scope)
            .bind(var_id.get_id())
            .execute(&mut *tr)
            .await
            .map_err(|e| {
                error!("[add_context_variable] failed to delete existing variable: {:?}", e);
                e
            })?;
        sqlx::query(
            r#"insert into fluxpro.process_instance_context_variable (
                        process_instance_uuid,
                        scope,
                        name,
                        value
                    )
                    values (
                        (select uuid from fluxpro.process_instance where token = $1),
                        $2,
                        $3,
                        $4
                    )
                    on conflict (process_instance_uuid, scope, name)
                    do update set
                        value = EXCLUDED.value;"#,
        )
        .bind(process_token.get_id())
        .bind(scope)
        .bind(var_id.get_id())
        .bind(json!(value))
        .execute(&mut *tr)
        .await
        .map_err(|e| {
            error!("[add_context_variable] failed to insert variable: {:?}", e);
            e
        })?;
        let _ = tr.commit().await.map_err(|e| {
            error!(
                "[add_context_variable] failed to commit transaction: {:?}",
                e
            );
            e
        })?;
        Ok(())
    }

    async fn get_process_def_node(
        &self,
        process_token: &IdField,
        node_id: &IdField,
    ) -> Result<Node, Error> {
        let found_node_def: JsonValue = sqlx::query_scalar(
            r#"select definition from fluxpro.process_def_node where
                    process_def_uuid=(select process_def_uuid from fluxpro.process_instance where token = $1) and node_id=$2"#
        ).bind(process_token.get_id())
            .bind(node_id.get_id())
            .fetch_one(&self.db_pool)
            .await?;
        let node =
            serde_json::from_value(found_node_def).map_err(|e| sqlx::Error::ColumnDecode {
                index: "definition".to_string(),
                source: Box::new(e),
            })?;
        Ok(node)
    }

    async fn get_process_def_node_uuid(
        &self,
        process_token: &IdField,
        node_id: &IdField,
    ) -> Result<Uuid, Error> {
        let node_uuid = sqlx::query_scalar(
            r#"select uuid from fluxpro.process_def_node where
                    process_def_uuid=(select process_def_uuid from fluxpro.process_instance where token = $1) and node_id=$2"#
        ).bind(process_token.get_id())
            .bind(node_id.get_id())
            .fetch_one(&self.db_pool)
            .await?;
        Ok(node_uuid)
    }

    async fn set_process_current_node(
        &self,
        process_token: &IdField,
        node: &Node,
    ) -> Result<(), Error> {
        sqlx::query(
            r#"
            update fluxpro.process_instance
                set revision=revision+1, node_visit_id=gen_random_uuid(), wait_completed=false, current_node_ref=
                    (select uuid from fluxpro.process_def_node
                                 where process_def_uuid = (
                        select process_def_uuid from fluxpro.process_instance where token=$2
                    ) and node_id=$1)
                where token=$2
            "#,
        )
        .bind(node.id().get_id())
        .bind(process_token.get_id())
        .execute(&self.db_pool)
        .await?;
        Ok(())
    }

    async fn add_queue_task<'a>(
        &self,
        queue_task: FluxproQueueTask,
        run_after: Option<DateTime<Utc>>,
        cancel_events: Option<Vec<QueueTaskCancel<'a>>>,
    ) -> Result<(), Error> {
        let mut tr = self.db_pool.begin().await?;
        let visit_id: Option<Uuid> =
            if let FluxproQueueTask::ProcessEvent { process_token, .. } = &queue_task {
                sqlx::query_scalar(
                    "select node_visit_id from fluxpro.process_instance where token=$1 for update",
                )
                .bind(process_token.get_id())
                .fetch_one(&mut *tr)
                .await?
            } else {
                None
            };
        let task_uuid: Uuid = sqlx::query_scalar(
            r#"insert into fluxpro.queue_runner (task, run_after, node_visit_id) values ($1, $2, $3) returning uuid"#,
        )
        .bind(serde_json::to_value(&queue_task).unwrap())
        .bind(run_after.unwrap_or_else(|| Utc::now()))
        .bind(visit_id)
        .fetch_one(&mut *tr)
        .await?;
        if let Some(cancel_events) = cancel_events {
            for cancel_event in cancel_events {
                sqlx::query(
                    r#"insert into fluxpro.queue_cancel_task (process_token, node_id, event_id, task_uuid) values ($1, $2, $3, $4)"#
                )
                    .bind(cancel_event.process_token().get_id())
                    .bind(cancel_event.node().id().get_id())
                    .bind(cancel_event.event_id().get_id())
                    .bind(task_uuid)
                    .execute(&mut *tr)
                    .await?;
            }
        }
        tr.commit().await?;
        Ok(())
    }

    async fn fetch_queue_task(
        &self,
        lock_key: &IdField,
        lease_ms: u64,
    ) -> anyhow::Result<Option<FluxproQueueTaskDefinition>> {
        let lease_ms = i64::try_from(lease_ms).unwrap_or(i64::MAX);
        anyhow::ensure!(lease_ms > 0, "task lease must be positive");
        let mut tx = self.db_pool.begin().await?;
        sqlx::query("set transaction isolation level read committed")
            .execute(&mut *tx)
            .await?;
        // Acquire the instance row first. A transaction advisory lock in a single
        // SELECT does not refresh that statement's MVCC snapshot after acquisition.
        // The second statement below sees any lease committed before this row lock.
        let token: Option<String> = sqlx::query_scalar(r#"
            select i.token from fluxpro.process_instance i
            join lateral (
                select q.run_after,q.created_at from fluxpro.queue_runner q
                where coalesce(q.task #>> '{process_node,process_token}',q.task #>> '{ProcessEvent,process_token}',q.task #>> '{ProcessSignal,process_token}')=i.token
                  and q.run_after<=clock_timestamp()
                  and (q.lock_key='' or q.locked_by is null or q.locked_by<clock_timestamp())
                  and (i.recovery_task_uuid is null or i.recovery_task_uuid=q.uuid)
                order by q.run_after,q.created_at,q.uuid limit 1
            ) ready on true
            where i.execution_state='running'
              and not exists (select 1 from fluxpro.queue_runner active
                  where coalesce(active.task #>> '{process_node,process_token}',active.task #>> '{ProcessEvent,process_token}',active.task #>> '{ProcessSignal,process_token}')=i.token
                    and active.lock_key<>'' and active.locked_by>=clock_timestamp())
            order by ready.run_after,ready.created_at,i.uuid
            for update of i skip locked limit 1
        "#).fetch_optional(&mut *tx).await?;
        let Some(token) = token else {
            tx.commit().await?;
            return Ok(None);
        };
        let task = sqlx::query_as(r#"
            with candidate as (
                select q.uuid from fluxpro.queue_runner q
                join fluxpro.process_instance i on i.token=$3
                where coalesce(q.task #>> '{process_node,process_token}',q.task #>> '{ProcessEvent,process_token}',q.task #>> '{ProcessSignal,process_token}')=$3
                  and i.execution_state='running'
                  and (i.recovery_task_uuid is null or i.recovery_task_uuid=q.uuid)
                  and q.run_after<=clock_timestamp()
                  and (q.lock_key='' or q.locked_by is null or q.locked_by<clock_timestamp())
                  and not exists (select 1 from fluxpro.queue_runner active
                    where coalesce(active.task #>> '{process_node,process_token}',active.task #>> '{ProcessEvent,process_token}',active.task #>> '{ProcessSignal,process_token}')=$3
                      and active.lock_key<>'' and active.locked_by>=clock_timestamp())
                order by q.run_after,q.created_at,q.uuid
                for update of q skip locked limit 1
            )
            update fluxpro.queue_runner q set lock_key=$1,locked_at=clock_timestamp(),
                locked_by=clock_timestamp()+($2*interval '1 millisecond'),attempts=attempts+1
            from candidate where q.uuid=candidate.uuid returning q.*
        "#).bind(lock_key.get_id()).bind(lease_ms).bind(token).fetch_optional(&mut *tx).await?;
        tx.commit().await?;
        Ok(task)
    }

    async fn get_queue_health(&self) -> anyhow::Result<FluxproQueueHealth> {
        sqlx::query_as(
            r#"
            select
                count(*) filter (
                    where run_after <= now()
                      and (lock_key = '' or locked_by is null or locked_by < now())
                ) as ready_tasks,
                count(*) filter (
                    where lock_key <> '' and locked_by >= clock_timestamp()
                ) as locked_tasks,
                min(run_after) filter (
                    where run_after <= now()
                      and (lock_key = '' or locked_by is null or locked_by < now())
                ) as oldest_ready_at
            from fluxpro.queue_runner
            "#,
        )
        .fetch_one(&self.db_pool)
        .await
        .map_err(Into::into)
    }

    async fn renew_queue_task_lock(
        &self,
        task_uuid: Uuid,
        lock_key: &str,
        lease_ms: u64,
    ) -> anyhow::Result<bool> {
        let lease_ms = i64::try_from(lease_ms).unwrap_or(i64::MAX);
        anyhow::ensure!(lease_ms > 0, "task lease must be positive");
        let mut tx = self.db_pool.begin().await?;
        // Check the clock after acquiring the row lock, not before a possible wait.
        let owned: Option<Uuid> = sqlx::query_scalar(
            "select uuid from fluxpro.queue_runner where uuid=$1 and lock_key=$2 for update",
        )
        .bind(task_uuid)
        .bind(lock_key)
        .fetch_optional(&mut *tx)
        .await?;
        let renewed = if owned.is_some() {
            sqlx::query("update fluxpro.queue_runner set locked_by=clock_timestamp()+($3*interval '1 millisecond') where uuid=$1 and lock_key=$2 and locked_by>clock_timestamp()")
                .bind(task_uuid).bind(lock_key).bind(lease_ms).execute(&mut *tx).await?.rows_affected()==1
        } else {
            false
        };
        tx.commit().await?;
        Ok(renewed)
    }

    async fn retry_queue_task(
        &self,
        task: &FluxproQueueTaskDefinition,
        run_after: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let updated = sqlx::query(
            r#"update fluxpro.queue_runner
                  set run_after = $3, lock_key = '', locked_at = null, locked_by = null
                where uuid = $1 and lock_key = $2 and locked_by >= clock_timestamp()"#,
        )
        .bind(task.uuid)
        .bind(&task.lock_key)
        .bind(run_after)
        .execute(&self.db_pool)
        .await?;
        anyhow::ensure!(
            updated.rows_affected() == 1,
            "queue task {} is no longer owned by lock {}",
            task.uuid,
            task.lock_key
        );
        Ok(())
    }

    async fn kill_queue_task(&self, task: &FluxproQueueTaskDefinition) -> anyhow::Result<()> {
        let mut tr = self.db_pool.begin().await?;
        sqlx::query(
            r#"delete from fluxpro.queue_cancel_task
               where task_uuid=$1
                 and exists (select 1 from fluxpro.queue_runner where uuid=$1 and lock_key=$2)"#,
        )
        .bind(task.uuid)
        .bind(&task.lock_key)
        .execute(&mut *tr)
        .await?;
        let deleted = sqlx::query("delete from fluxpro.queue_runner where uuid=$1 and lock_key=$2")
            .bind(task.uuid)
            .bind(&task.lock_key)
            .execute(&mut *tr)
            .await?;
        if deleted.rows_affected() != 1 {
            return Err(anyhow::anyhow!(
                "queue task {} is no longer owned by lock {}",
                task.uuid,
                task.lock_key
            ));
        }
        tr.commit().await?;
        Ok(())
    }

    async fn create_event_schedule(
        &self,
        process_token: &IdField,
        node: &Node,
        when: DateTime<Utc>,
        on_time: &IdField,
    ) -> anyhow::Result<()> {
        sqlx::query(
            r#"
            insert into fluxpro.event_schedule (
                process_instance_uuid, process_def_node_uuid, scheduled_time, on_time
            ) values ((select uuid from fluxpro.process_instance where token = $1),
                      (select uuid from fluxpro.process_def_node where process_def_uuid = (
                          select process_def_uuid from fluxpro.process_instance where token=$1
                      ) and node_id=$2),
                      $3, $4)"#,
        )
        .bind(process_token.get_id())
        .bind(node.id().get_id())
        .bind(when)
        .bind(on_time.get_id())
        .execute(&self.db_pool)
        .await?;
        Ok(())
    }

    async fn get_process_instance_node(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<Option<Node>> {
        let node_def: Option<serde_json::Value> = sqlx::query_scalar(
            "select n.definition from fluxpro.process_instance i \
             left join fluxpro.process_def_node n on n.uuid=i.current_node_ref where i.token=$1",
        )
        .bind(process_token.get_id())
        .fetch_one(&self.db_pool)
        .await?;
        node_def
            .map(serde_json::from_value::<Node>)
            .transpose()
            .map_err(Into::into)
    }

    async fn get_process_instance_context(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<ContextMap> {
        let context_vars: Vec<ProcessInstanceContextVariableDb> = sqlx::query_as(
            r#"select * from fluxpro.process_instance_context_variable
                where process_instance_uuid=(select uuid from fluxpro.process_instance where token = $1)"#
        )
            .bind(process_token.get_id())
            .fetch_all(&self.db_pool)
            .await?;
        let mut context = HashMap::new();
        context_vars.iter().for_each(|var| {
            context.insert(var.name.clone(), var.value.clone());
        });
        Ok(ContextMap(context))
    }

    async fn save_process_instance_context(
        &self,
        process_token: &IdField,
        context: &ContextMap,
    ) -> anyhow::Result<()> {
        let mut tr = self.db_pool.begin().await?;
        sqlx::query("update fluxpro.process_instance set revision=revision+1 where token=$1")
            .bind(process_token.get_id())
            .execute(&mut *tr)
            .await?;
        // Merge supplied keys; omitted keys remain stored, including keys removed locally.
        for (key, value) in context.0.iter() {
            sqlx::query(
                r#"delete from fluxpro.process_instance_context_variable where process_instance_uuid=(select uuid from fluxpro.process_instance where token = $1) and scope=$2 and name=$3"#
            )
                .bind(process_token.get_id())
                .bind("_")
                .bind(key.get_id())
                .execute(&mut *tr)
                .await?;
            sqlx::query(
                r#"insert into fluxpro.process_instance_context_variable (process_instance_uuid, scope, name, value)
                    VALUES ((select uuid from fluxpro.process_instance where token=$1), $2, $3, $4)"#
            ).bind(process_token.get_id())
                .bind("_")
                .bind(key.get_id())
                .bind(json!(value))
                .execute(&mut *tr)
                .await?;
        }
        tr.commit().await?;
        Ok(())
    }

    async fn cancel_queue_tasks(
        &self,
        node_id: &IdField,
        signal_id: &IdField,
        process_token: &IdField,
    ) -> anyhow::Result<()> {
        let task_uuid: Vec<Uuid> = sqlx::query_scalar(
            r#"select task_uuid from fluxpro.queue_cancel_task where node_id=$1 and event_id=$2 and process_token=$3"#
        )
            .bind(node_id.get_id())
            .bind(signal_id.get_id())
            .bind(process_token.get_id())
            .fetch_all(&self.db_pool)
            .await?;
        let mut tr = self.db_pool.begin().await?;
        sqlx::query("delete from fluxpro.queue_cancel_task where task_uuid = any($1)")
            .bind(&task_uuid)
            .execute(&mut *tr)
            .await?;
        sqlx::query("delete from fluxpro.queue_runner where uuid = any($1)")
            .bind(&task_uuid)
            .execute(&mut *tr)
            .await?;
        tr.commit().await?;
        Ok(())
    }

    async fn get_process_token_by_process_id(
        &self,
        process_id: &IdField,
    ) -> anyhow::Result<IdField> {
        let found_token: String =
            sqlx::query_scalar(r#"select token from fluxpro.process_instance where process_id=$1"#)
                .bind(process_id.get_id())
                .fetch_one(&self.db_pool)
                .await?;
        Ok(IdField::new(&found_token)?)
    }

    async fn get_escalation_actions(
        &self,
        process_token: &IdField,
        escalation_topic: &IdField,
    ) -> anyhow::Result<Vec<EscalationActionDef>> {
        let definition: Option<JsonValue> = sqlx::query_scalar(
            r#"
        select e.definition
        from fluxpro.process_def_escalations e
        where e.escalation_id = $1
          and e.process_def_uuid = (
              select i.process_def_uuid
              from fluxpro.process_instance i
              where i.token = $2
          )
        "#,
        )
        .bind(escalation_topic.get_id())
        .bind(process_token.get_id())
        .fetch_optional(&self.db_pool)
        .await
        .map_err(|e| {
            error!(
                "[get_escalation_actions] db_error escalation_id={} token={} err={}",
                escalation_topic.get_id(),
                process_token.get_id(),
                e
            );
            anyhow::Error::from(e)
        })?;

        let Some(definition) = definition else {
            return Ok(vec![]);
        };

        let escalation: EscalationDef = serde_json::from_value(definition).with_context(|| {
            format!(
                "Invalid EscalationDef JSON: escalation_id={} token={}",
                escalation_topic.get_id(),
                process_token.get_id(),
            )
        })?;

        Ok(escalation.actions)
    }

    async fn log_signal(&self, process_token: &IdField, signal: &PostSignal) -> anyhow::Result<()> {
        sqlx::query(
            r#"insert into fluxpro.signal_history (process_id, signal_name, payload)
                        values ($1, $2, $3)"#,
        )
        .bind(process_token.get_id())
        .bind(signal.signal.get_id())
        .bind(json!(signal.context.0))
        .execute(&self.db_pool)
        .await
        .map_err(|e| anyhow::Error::msg(format!("failed to log signal: {:?}", e)))?;
        Ok(())
    }

    async fn get_last_signal(&self, process_token: &IdField) -> anyhow::Result<Option<PostSignal>> {
        let row = sqlx::query(
            r#"
                select signal_name, payload, event_id, wait_visit_id
                from fluxpro.signal_history
                where process_id = $1
                order by created_at desc, uuid desc
                limit 1
                "#,
        )
        .bind(process_token.get_id())
        .fetch_optional(&self.db_pool)
        .await?;

        row.map(signal_from_row).transpose()
    }

    async fn enqueue_signal_if_new(
        &self,
        process_token: &IdField,
        signal: &PostSignal,
    ) -> anyhow::Result<bool> {
        let mut transaction = self.db_pool.begin().await?;

        // Serialize signal admission for one process. Without this lock, concurrent
        // callers could race to insert the same producer event identity.
        let instance = sqlx::query(
            "select i.uuid, i.node_visit_id, n.definition from fluxpro.process_instance i \
             left join fluxpro.process_def_node n on n.uuid=i.current_node_ref \
             where i.token=$1 for update of i",
        )
        .bind(process_token.get_id())
        .fetch_one(&mut *transaction)
        .await?;
        let current_node = instance
            .try_get::<Option<serde_json::Value>, _>("definition")?
            .map(serde_json::from_value::<Node>)
            .transpose()?;
        let visit_id: Option<Uuid> =
            if task_execution::accepts_signal(current_node.as_ref(), &signal.signal) {
                instance.try_get("node_visit_id")?
            } else {
                None
            };

        let visit_id = signal.wait_visit_id.or(visit_id);
        if let Some(event_id) = &signal.event_id {
            anyhow::ensure!(
                !event_id.trim().is_empty() && event_id.chars().count() <= 256,
                "event_id must contain 1 to 256 characters"
            );
            let instance_id: Uuid = instance.try_get("uuid")?;
            let receipt = sqlx::query("select signal,payload,requested_visit from fluxpro.signal_receipt where process_instance_uuid=$1 and event_id=$2")
                .bind(instance_id).bind(event_id).fetch_optional(&mut *transaction).await?;
            if let Some(receipt) = receipt {
                anyhow::ensure!(
                    receipt.try_get::<String, _>("signal")? == signal.signal.get_id()
                        && receipt.try_get::<JsonValue, _>("payload")? == json!(signal.context)
                        && receipt.try_get::<Option<Uuid>, _>("requested_visit")?
                            == signal.wait_visit_id,
                    "event_id was already used for a different signal, payload, or wait visit"
                );
                transaction.commit().await?;
                return Ok(false);
            }
            sqlx::query("insert into fluxpro.signal_receipt(process_instance_uuid,event_id,signal,payload,requested_visit) values($1,$2,$3,$4,$5)")
                .bind(instance_id).bind(event_id).bind(signal.signal.get_id()).bind(json!(signal.context)).bind(signal.wait_visit_id)
                .execute(&mut *transaction).await?;
        }

        sqlx::query(
            r#"insert into fluxpro.signal_history (process_id, signal_name, payload, event_id, wait_visit_id)
               values ($1, $2, $3, $4, $5)"#,
        )
        .bind(process_token.get_id())
        .bind(signal.signal.get_id())
        .bind(json!(signal.context.0))
        .bind(&signal.event_id)
        .bind(signal.wait_visit_id)
        .execute(&mut *transaction)
        .await?;

        sqlx::query(r#"insert into fluxpro.queue_runner (task, run_after, node_visit_id) values ($1, $2, $3)"#)
            .bind(serde_json::to_value(FluxproQueueTask::ProcessSignal {
                process_token: process_token.clone(),
                signal: signal.clone(),
            })?)
            .bind(Utc::now())
            .bind(visit_id)
            .execute(&mut *transaction)
            .await?;

        transaction.commit().await?;
        Ok(true)
    }
}

fn signal_from_row(row: sqlx::postgres::PgRow) -> anyhow::Result<PostSignal> {
    let signal_name: String = row.try_get("signal_name")?;
    let signal = IdField::new(signal_name)
        .map_err(|_| anyhow::Error::msg("failed to create IdField from signal_name"))?;
    let payload: sqlx::types::Json<ContextMap> = row.try_get("payload")?;
    Ok(PostSignal {
        event_id: row.try_get("event_id")?,
        wait_visit_id: row.try_get("wait_visit_id")?,
        signal,
        context: payload.0,
    })
}

impl FluxproDbServiceImpl {
    async fn register_process_definition(
        &self,
        process_def: &ProcessDefinition,
        compiled_process_def: JsonValue,
        source_definition: Option<&str>,
        publication: bool,
        base_uuid: Option<Uuid>,
    ) -> Result<Uuid, sqlx::Error> {
        let mut tr = self.db_pool.begin().await?;
        // Serialize registration per key so insertion order is deterministic.
        sqlx::query("select pg_advisory_xact_lock(hashtextextended($1, 1))")
            .bind(process_def.key.get_id())
            .execute(&mut *tr)
            .await?;
        if publication {
            let versions: Vec<(Uuid, String)> = sqlx::query_as(
                "select uuid, version from fluxpro.process_definition where key_=$1",
            )
            .bind(process_def.key.get_id())
            .fetch_all(&mut *tr)
            .await?;
            if let Some(base) = base_uuid {
                if !versions.iter().any(|(uuid, _)| *uuid == base) {
                    return Err(sqlx::Error::Protocol(
                        "The original version does not belong to this process key".into(),
                    ));
                }
                for (_, version) in &versions {
                    let version = VersionId::new(version).map_err(sqlx::Error::Protocol)?;
                    if process_def.version <= version {
                        return Err(sqlx::Error::Protocol(format!(
                            "Choose a version newer than {version}; refresh if another publication has completed"
                        )));
                    }
                }
            } else if !versions.is_empty() {
                return Err(sqlx::Error::Protocol(
                    "This process key already exists. Open it to publish a new version".into(),
                ));
            }
        }
        let index_id: i32 = sqlx::query_scalar(
            r#"select coalesce(max(index_id), 0) from fluxpro.process_definition where key_=$1"#,
        )
        .bind(process_def.key.get_id())
        .fetch_one(&mut *tr)
        .await?;
        let process_def_uuid: Uuid = sqlx::query_scalar(
            r#"INSERT INTO
                    fluxpro.process_definition
                        (key_, version, index_id, status, effective_from, deprecated_at,
                         version_comment, definition, source_definition)
                        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING uuid"#,
        )
        .bind(process_def.key.get_id())
        .bind(process_def.version.as_str())
        .bind(index_id + 1)
        .bind("Draft")
        .bind(process_def.effective_from.unwrap_or_else(Utc::now))
        .bind(process_def.deprecated_at)
        .bind(process_def.metadata.comment.as_deref())
        .bind(compiled_process_def)
        .bind(source_definition)
        .fetch_one(&mut *tr)
        .await?;
        for node in &process_def.nodes {
            sqlx::query(
                r#"insert into fluxpro.process_def_node
                    (process_def_uuid, node_id, definition) values ($1, $2, $3)
                "#,
            )
            .bind(process_def_uuid)
            .bind(node.id().get_id())
            .bind(json!(node))
            .execute(&mut *tr)
            .await?;
        }
        for signal in &process_def.signals {
            sqlx::query(
                r#"insert into fluxpro.process_def_signal (
                    process_def_uuid, signal_id
                ) values ($1, $2)
                "#,
            )
            .bind(process_def_uuid)
            .bind(signal.name())
            .execute(&mut *tr)
            .await?;
        }
        for escalation in &process_def.escalations {
            sqlx::query(
                r#"insert into fluxpro.process_def_escalations (
                    process_def_uuid, escalation_id, definition
                ) values ($1, $2, $3)
                "#,
            )
            .bind(process_def_uuid)
            .bind(escalation.topic.get_id())
            .bind(json!(escalation))
            .execute(&mut *tr)
            .await?;
        }
        for stage in &process_def.stages {
            match stage {
                StageDef::Obj {
                    id,
                    name,
                    is_initial,
                    is_final,
                } => {
                    sqlx::query(r#"insert into fluxpro.process_stage
                            (process_def_uuid, stage_id, name, is_initial, is_final) VALUES ($1, $2, $3, $4, $5)
                    "#).bind(process_def_uuid)
                        .bind(id.get_id())
                        .bind(name)
                        .bind(is_initial)
                        .bind(is_final)
                        .execute(&mut *tr)
                        .await?;
                }
                StageDef::Name(id) => {
                    sqlx::query(r#"insert into fluxpro.process_stage
                            (process_def_uuid, stage_id, name, is_initial, is_final) VALUES ($1, $2, $3, $4, $5)
                    "#).bind(process_def_uuid)
                        .bind(id.get_id())
                        .bind(id.get_id())
                        .bind(false)
                        .bind(false)
                        .execute(&mut *tr)
                        .await?;
                }
            }
        }
        sqlx::query("update fluxpro.process_definition set status=$2 where uuid=$1")
            .bind(process_def_uuid)
            .bind(process_def.status.to_string())
            .execute(&mut *tr)
            .await?;
        tr.commit().await?;
        Ok(process_def_uuid)
    }
}
