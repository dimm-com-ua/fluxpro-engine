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

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct FluxproQueueHealth {
    pub ready_tasks: i64,
    pub locked_tasks: i64,
    pub oldest_ready_at: Option<DateTime<Utc>>,
}

#[async_trait]
pub trait FluxproDbService {
    async fn create_service_process_def(
        &self,
        process_def: &ProcessDefinition,
        compiled_process_def: JsonValue,
        source_definition: Option<&str>,
    ) -> Result<Uuid, sqlx::Error>;
    async fn get_current_process_def(
        &self,
        process_def_id: IdField,
        version: Option<VersionId>,
    ) -> Result<ProcessDefDb, sqlx::Error>;
    async fn get_process_def_by_instance(
        &self,
        process_token: &IdField,
    ) -> Result<ProcessDefDb, sqlx::Error>;
    async fn create_process_instance(
        &self,
        process_def_uuid: Uuid,
        process_id: IdField,
    ) -> Result<IdField, sqlx::Error>;
    async fn check_process_id(
        &self,
        process_def_uuid: Uuid,
        process_id: &IdField,
    ) -> Result<bool, sqlx::Error>;
    async fn get_stage_uuid(
        &self,
        process_token: &IdField,
        stage_id: &IdField,
    ) -> Result<Uuid, sqlx::Error>;

    async fn set_process_instance_stage(
        &self,
        process_def_uuid: &IdField,
        stage_uuid: Uuid,
        reason: Option<String>,
    ) -> Result<(), sqlx::Error>;
    async fn add_context_variable(
        &self,
        process_token: &IdField,
        scope: &str,
        var_id: &IdField,
        value: &ContextValue,
    ) -> Result<(), sqlx::Error>;
    async fn get_process_def_node(
        &self,
        process_token: &IdField,
        node_id: &IdField,
    ) -> Result<Node, sqlx::Error>;
    async fn get_process_def_node_uuid(
        &self,
        process_token: &IdField,
        node_id: &IdField,
    ) -> Result<Uuid, sqlx::Error>;
    async fn set_process_current_node(
        &self,
        process_token: &IdField,
        node: &Node,
    ) -> Result<(), sqlx::Error>;
    async fn add_queue_task<'a>(
        &self,
        queue_task: FluxproQueueTask,
        run_after: Option<DateTime<Utc>>,
        cancel_events: Option<Vec<QueueTaskCancel<'a>>>,
    ) -> Result<(), sqlx::Error>;
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
    async fn kill_queue_task(&self, task: &FluxproQueueTaskDefinition) -> anyhow::Result<()>;
    async fn create_event_schedule(
        &self,
        process_token: &IdField,
        node: &Node,
        when: DateTime<Utc>,
        on_time: &IdField,
    ) -> anyhow::Result<()>;
    async fn get_process_instance_node(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<Option<Node>>;
    async fn get_process_instance_context(
        &self,
        process_token: &IdField,
    ) -> anyhow::Result<ContextMap>;
    async fn save_process_instance_context(
        &self,
        process_token: &IdField,
        context: &ContextMap,
    ) -> anyhow::Result<()>;
    async fn cancel_queue_tasks(
        &self,
        node_id: &IdField,
        signal_id: &IdField,
        process_token: &IdField,
    ) -> anyhow::Result<()>;
    async fn get_process_token_by_process_id(
        &self,
        process_id: &IdField,
    ) -> anyhow::Result<IdField>;
    async fn get_escalation_actions(
        &self,
        process_token: &IdField,
        escalation_topic: &IdField,
    ) -> anyhow::Result<Vec<EscalationActionDef>>;
    async fn log_signal(&self, process_token: &IdField, signal: &PostSignal) -> anyhow::Result<()>;
    async fn get_last_signal(&self, process_token: &IdField) -> anyhow::Result<Option<PostSignal>>;
    /// Persists a signal and enqueues its delivery as one operation.
    ///
    /// Implementations should override this method when they can provide transactional
    /// serialization per process instance. The default keeps custom implementations
    /// source-compatible, but cannot prevent concurrent duplicate submissions.
    async fn enqueue_signal_if_new(
        &self,
        process_token: &IdField,
        signal: &PostSignal,
    ) -> anyhow::Result<bool> {
        if is_duplicate_signal(self.get_last_signal(process_token).await?.as_ref(), signal) {
            return Ok(false);
        }
        self.log_signal(process_token, signal).await?;
        self.add_queue_task(
            FluxproQueueTask::ProcessSignal {
                process_token: process_token.clone(),
                signal: signal.clone(),
            },
            None,
            None,
        )
        .await?;
        Ok(true)
    }
    async fn write_execution_log(
        &self,
        process_token: &IdField,
        event: &ExecutionLogEvent,
    ) -> anyhow::Result<()>;
}

pub struct FluxproDbServiceImpl {
    db_pool: Pool<Postgres>,
}

impl FluxproDbServiceImpl {
    pub fn new(db_pool: Pool<Postgres>) -> Self {
        Self { db_pool }
    }
}

#[async_trait]
impl FluxproDbService for FluxproDbServiceImpl {
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
        let mut tr = self.db_pool.begin().await?;
        let index_id = sqlx::query_scalar(
            r#"select max(index_id) from fluxpro.process_definition where key_=$1"#,
        )
        .bind(process_def.key.get_id())
        .fetch_one(&mut *tr)
        .await
        .map_err(|e| {
            error!(
                "[create_service_process_def] error while fetching the current max index_id: {}",
                e
            );
            e
        })
        .unwrap_or(0);
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
        .bind(process_def.status.to_string())
        .bind(process_def.effective_from)
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
        let _ = tr.commit().await?;
        Ok(process_def_uuid)
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
                set current_node_ref=
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
        let task_uuid: Uuid = sqlx::query_scalar(
            r#"insert into fluxpro.queue_runner (task, run_after) values ($1, $2) returning uuid"#,
        )
        .bind(serde_json::to_value(&queue_task).unwrap())
        .bind(run_after.unwrap_or_else(|| Utc::now()))
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
        sqlx::query_as(
            r#"
            with candidate as (
                select pending.uuid
                from fluxpro.queue_runner pending
                where pending.run_after <= now()
                  and (pending.lock_key = '' or pending.locked_by is null or pending.locked_by < now())
                  -- One workflow instance is processed serially. The advisory
                  -- transaction lock closes the race where two workers select
                  -- different ready rows for the same process before either row's
                  -- lease update becomes visible.
                  and pg_try_advisory_xact_lock(hashtextextended(coalesce(
                        pending.task #>> '{process_node,process_token}',
                        pending.task #>> '{ProcessEvent,process_token}',
                        pending.task #>> '{ProcessSignal,process_token}'
                      ), 0))
                  and not exists (
                      select 1
                      from fluxpro.queue_runner active
                      where active.uuid <> pending.uuid
                        and active.lock_key <> ''
                        and active.locked_by >= now()
                        and coalesce(
                              active.task #>> '{process_node,process_token}',
                              active.task #>> '{ProcessEvent,process_token}',
                              active.task #>> '{ProcessSignal,process_token}'
                            ) = coalesce(
                              pending.task #>> '{process_node,process_token}',
                              pending.task #>> '{ProcessEvent,process_token}',
                              pending.task #>> '{ProcessSignal,process_token}'
                            )
                  )
                order by pending.run_after, pending.created_at
                for update skip locked
                limit 1
            )
            update fluxpro.queue_runner q
               set lock_key = $1,
                   locked_at = now(),
                   locked_by = now() + ($2 * interval '1 millisecond'),
                   attempts = attempts + 1
              from candidate
             where q.uuid = candidate.uuid
            returning q.*
            "#,
        )
        .bind(lock_key.get_id())
        .bind(lease_ms)
        .fetch_optional(&self.db_pool)
        .await
        .map_err(|e| {
            error!("error while atomically fetching a queue task: {}", e);
            e.into()
        })
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
                    where lock_key <> '' and locked_by >= now()
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
        let result = sqlx::query(
            r#"
            update fluxpro.queue_runner
               set locked_by = now() + ($3 * interval '1 millisecond')
             where uuid = $1 and lock_key = $2 and locked_by >= now()
            "#,
        )
        .bind(task_uuid)
        .bind(lock_key)
        .bind(lease_ms)
        .execute(&self.db_pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn retry_queue_task(
        &self,
        task: &FluxproQueueTaskDefinition,
        run_after: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let updated = sqlx::query(
            r#"update fluxpro.queue_runner
                  set run_after = $3, lock_key = '', locked_at = null, locked_by = null
                where uuid = $1 and lock_key = $2 and locked_by >= now()"#,
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
        let node_def = sqlx::query_scalar(
            r#"
            select definition from fluxpro.process_def_node where
                    uuid=(select current_node_ref from fluxpro.process_instance where token = $1)"#,
        )
        .bind(process_token.get_id())
        .fetch_optional(&self.db_pool)
        .await?;
        if let Some(node_def) = node_def {
            if let Ok(node) = serde_json::from_value::<Node>(node_def) {
                return Ok(Some(node));
            }
        }
        Err(anyhow::Error::msg("Node not found"))
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
        // sqlx::query(
        //     r#"delete from fluxpro.process_instance_context_variable where process_instance_uuid=(select uuid from fluxpro.process_instance where token = $1)"#
        // )
        //     .bind(process_token.get_id())
        //     .execute(&mut *tr)
        //     .await?;
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
                select signal_name, payload
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
        // callers can all observe the same previous signal and enqueue duplicate paths.
        let process_exists = sqlx::query_scalar::<_, Uuid>(
            r#"select uuid from fluxpro.process_instance where token = $1 for update"#,
        )
        .bind(process_token.get_id())
        .fetch_optional(&mut *transaction)
        .await?;
        anyhow::ensure!(
            process_exists.is_some(),
            "process instance {} not found",
            process_token
        );

        let last_signal = sqlx::query(
            r#"
                select signal_name, payload
                from fluxpro.signal_history
                where process_id = $1
                order by created_at desc, uuid desc
                limit 1
                "#,
        )
        .bind(process_token.get_id())
        .fetch_optional(&mut *transaction)
        .await?
        .map(signal_from_row)
        .transpose()?;

        if is_duplicate_signal(last_signal.as_ref(), signal) {
            transaction.commit().await?;
            return Ok(false);
        }

        sqlx::query(
            r#"insert into fluxpro.signal_history (process_id, signal_name, payload)
               values ($1, $2, $3)"#,
        )
        .bind(process_token.get_id())
        .bind(signal.signal.get_id())
        .bind(json!(signal.context.0))
        .execute(&mut *transaction)
        .await?;

        sqlx::query(r#"insert into fluxpro.queue_runner (task, run_after) values ($1, $2)"#)
            .bind(serde_json::to_value(FluxproQueueTask::ProcessSignal {
                process_token: process_token.clone(),
                signal: signal.clone(),
            })?)
            .bind(Utc::now())
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
        signal,
        context: payload.0,
    })
}

fn is_duplicate_signal(last_signal: Option<&PostSignal>, signal: &PostSignal) -> bool {
    last_signal == Some(signal)
}

#[cfg(test)]
mod signal_deduplication_tests {
    use super::*;
    use crate::models::context_map::context_map::ContextValue;

    fn signal(name: &str, otp: &str) -> PostSignal {
        PostSignal {
            signal: IdField::new(name).unwrap(),
            context: ContextMap(HashMap::from([(
                IdField::new("otp").unwrap(),
                ContextValue::string(otp.to_string()),
            )])),
        }
    }

    #[test]
    fn identical_signal_and_context_are_duplicates() {
        let first = signal("otp_verified", "1234");
        let repeated = first.clone();

        assert!(is_duplicate_signal(Some(&first), &repeated));
    }

    #[test]
    fn changed_context_is_a_new_signal() {
        let first = signal("otp_verified", "1234");
        let next = signal("otp_verified", "5678");

        assert!(!is_duplicate_signal(Some(&first), &next));
    }
}
