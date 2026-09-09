//! Atomic task snapshots, fenced transition commits, and instance startup.

use super::FluxproDbServiceImpl;
use crate::models::commands::start_process_instance::StartProcessInstance;
use crate::models::context_map::context_map::{ContextMap, ContextValue};
use crate::models::context_map::context_patcher::{ContextPatcher, ContextPatcherOp};
use crate::models::execution_log::ExecutionLogEvent;
use crate::models::id_field::IdField;
use crate::models::process_def::{Node, ProcessDefinition, SetStageWithReason};
use crate::models::queue::queue_task::{FluxproQueueTask, FluxproQueueTaskDefinition};
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Postgres, Row, Transaction};
use uuid::Uuid;

/// Consistent input for an execution attempt, read while its lease is valid.
#[derive(Clone)]
pub struct TaskExecutionSnapshot {
    /// Instance revision used to reject stale execution results.
    pub revision: i64,
    /// Identity of the current node visit, distinct from its workflow node ID.
    pub visit_id: Option<Uuid>,
    /// Whether an event has already completed the current wait.
    pub wait_completed: bool,
    /// Wait visit bound to this timeout or signal, if any.
    pub task_visit_id: Option<Uuid>,
    /// Definition version permanently associated with this instance.
    pub definition: ProcessDefinition,
    /// Persisted current node; absent before the first node entry.
    pub current_node: Option<Node>,
    /// Context visible to this execution attempt.
    pub context: ContextMap,
}

/// A successor or timeout to insert in the same transaction as its predecessor completes.
pub struct ScheduledTask {
    /// Existing wire-compatible task payload.
    pub task: FluxproQueueTask,
    /// Earliest execution time; `None` means immediately.
    pub run_after: Option<DateTime<Utc>>,
    /// Originating visit for timeout events; node tasks leave this unset.
    pub visit_id: Option<Uuid>,
    /// Signals which cancel this timeout.
    pub cancel_signals: Vec<IdField>,
}

/// Database effects prepared without holding a transaction across host handler calls.
#[derive(Default)]
pub struct TaskExecutionChanges {
    /// Node to enter, using the source queue UUID as its stable visit ID.
    pub enter_node: Option<IdField>,
    /// Optional stage assignment and its reason.
    pub stage: Option<SetStageWithReason>,
    /// Context keys to merge into the default scope.
    pub context: Option<ContextMap>,
    /// Ordered handler changes; removals delete stored keys in the default scope.
    pub context_patch: Option<ContextPatcher>,
    /// Stops execution and retains this task with a durable incident (kind, reason).
    pub incident: Option<(String, String)>,
    /// Consumes the current wait and cancels its remaining timeout tasks.
    pub close_wait: bool,
    /// Successors and timeouts committed with this transition.
    pub tasks: Vec<ScheduledTask>,
    /// Durable events committed with their corresponding state changes.
    pub events: Vec<ExecutionLogEvent>,
    /// Retains and releases the source task for retry instead of deleting it.
    pub retry_at: Option<DateTime<Utc>>,
}

async fn lock_task(
    tx: &mut Transaction<'_, Postgres>,
    task: &FluxproQueueTaskDefinition,
) -> anyhow::Result<Option<Uuid>> {
    let row = sqlx::query(
        "select node_visit_id, task from fluxpro.queue_runner \
         where uuid=$1 and lock_key=$2 and locked_by > clock_timestamp() for update",
    )
    .bind(task.uuid)
    .bind(&task.lock_key)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| anyhow::anyhow!("task {} no longer has a valid lease", task.uuid))?;
    let stored: FluxproQueueTask = serde_json::from_value(row.try_get("task")?)?;
    anyhow::ensure!(
        serde_json::to_value(stored)? == serde_json::to_value(&task.task)?,
        "task payload changed after dequeue"
    );
    Ok(row.try_get("node_visit_id")?)
}

pub(super) async fn load(
    db: &FluxproDbServiceImpl,
    task: &FluxproQueueTaskDefinition,
) -> anyhow::Result<TaskExecutionSnapshot> {
    let mut tx = db.db_pool.begin().await?;
    // All transition operations lock the instance before the queue row.
    let instance = sqlx::query(
        "select execution_state, revision, node_visit_id, wait_completed, current_node_ref, process_def_uuid \
         from fluxpro.process_instance where token=$1 for update",
    )
    .bind(task.task.process_token().get_id())
    .fetch_one(&mut *tx)
    .await?;
    anyhow::ensure!(
        instance.try_get::<String, _>("execution_state")? == "running",
        "instance is suspended"
    );
    let mut task_visit_id = lock_task(&mut tx, task).await?;
    let definition_uuid: Uuid = instance.try_get("process_def_uuid")?;
    let raw: serde_json::Value =
        sqlx::query_scalar("select definition from fluxpro.process_definition where uuid=$1")
            .bind(definition_uuid)
            .fetch_one(&mut *tx)
            .await?;
    let mut definition: ProcessDefinition = serde_json::from_value(raw)?;
    definition.uuid = Some(definition_uuid);
    let current_ref: Option<Uuid> = instance.try_get("current_node_ref")?;
    let current_node = if let Some(current_ref) = current_ref {
        let raw = sqlx::query_scalar::<_, serde_json::Value>(
            "select definition from fluxpro.process_def_node where uuid=$1 and process_def_uuid=$2",
        )
        .bind(current_ref)
        .bind(definition_uuid)
        .fetch_one(&mut *tx)
        .await?;
        Some(serde_json::from_value::<Node>(raw)?)
    } else {
        None
    };
    let visit_id: Option<Uuid> = instance.try_get("node_visit_id")?;
    if task_visit_id.is_none()
        && let FluxproQueueTask::ProcessSignal { signal, .. } = &task.task
        && accepts_signal(current_node.as_ref(), &signal.signal)
    {
        task_visit_id = visit_id;
        sqlx::query("update fluxpro.queue_runner set node_visit_id=$2 where uuid=$1")
            .bind(task.uuid)
            .bind(task_visit_id)
            .execute(&mut *tx)
            .await?;
    }
    let rows = sqlx::query(
        "select name, value from fluxpro.process_instance_context_variable \
         where process_instance_uuid=(select uuid from fluxpro.process_instance where token=$1)",
    )
    .bind(task.task.process_token().get_id())
    .fetch_all(&mut *tx)
    .await?;
    let mut context = ContextMap::default();
    for row in rows {
        context.0.insert(
            IdField::new(row.try_get::<String, _>("name")?)?,
            serde_json::from_value::<ContextValue>(row.try_get("value")?)?,
        );
    }
    let snapshot = TaskExecutionSnapshot {
        revision: instance.try_get("revision")?,
        visit_id,
        wait_completed: instance.try_get("wait_completed")?,
        task_visit_id,
        definition,
        current_node,
        context,
    };
    tx.commit().await?;
    Ok(snapshot)
}

pub(super) fn accepts_signal(node: Option<&Node>, signal: &IdField) -> bool {
    match node {
        Some(Node::UserTask {
            wait_for: Some(wait),
            ..
        })
        | Some(Node::Wait { wait_for: wait, .. }) => wait.signals().contains(signal),
        _ => false,
    }
}

pub(super) async fn apply_patch(
    connection: &mut PgConnection,
    instance_uuid: Uuid,
    patch: &ContextPatcher,
) -> anyhow::Result<()> {
    for op in patch.ops() {
        match op {
            ContextPatcherOp::Set { key, value } => {
                merge_context(
                    &mut *connection,
                    instance_uuid,
                    &ContextMap([(key.clone(), value.clone())].into()),
                )
                .await?;
            }
            ContextPatcherOp::Remove { key } => {
                sqlx::query("delete from fluxpro.process_instance_context_variable where process_instance_uuid=$1 and scope='_' and name=$2")
                        .bind(instance_uuid).bind(key.get_id()).execute(&mut *connection).await?;
            }
        }
    }
    Ok(())
}

async fn merge_context(
    connection: &mut PgConnection,
    instance_uuid: Uuid,
    context: &ContextMap,
) -> anyhow::Result<()> {
    for (key, value) in &context.0 {
        sqlx::query(
            "insert into fluxpro.process_instance_context_variable \
             (process_instance_uuid, scope, name, value) values ($1, '_', $2, $3) \
             on conflict (process_instance_uuid, scope, name) do update set value=excluded.value",
        )
        .bind(instance_uuid)
        .bind(key.get_id())
        .bind(serde_json::to_value(value)?)
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

async fn assign_stage(
    connection: &mut PgConnection,
    instance_uuid: Uuid,
    definition_uuid: Uuid,
    stage: &SetStageWithReason,
) -> anyhow::Result<()> {
    let stage_uuid: Uuid = sqlx::query_scalar(
        "select uuid from fluxpro.process_stage where process_def_uuid=$1 and stage_id=$2",
    )
    .bind(definition_uuid)
    .bind(stage.stage.get_id())
    .fetch_one(&mut *connection)
    .await?;
    sqlx::query("update fluxpro.process_instance set current_stage=$2, current_stage_reason=$3 where uuid=$1")
        .bind(instance_uuid).bind(stage_uuid).bind(&stage.reason).execute(&mut *connection).await?;
    sqlx::query("insert into fluxpro.process_instance_stage_log (process_instance_uuid, stage_uuid, reason) values ($1,$2,$3)")
        .bind(instance_uuid).bind(stage_uuid).bind(&stage.reason).execute(&mut *connection).await?;
    Ok(())
}

async fn enqueue(connection: &mut PgConnection, pending: &ScheduledTask) -> anyhow::Result<()> {
    let target = match &pending.task {
        FluxproQueueTask::ProcessNode { node, .. } => Some(node.id()),
        FluxproQueueTask::ProcessEvent { on_time, .. } => Some(on_time),
        FluxproQueueTask::ProcessSignal { .. } => None,
    };
    if let Some(target) = target {
        let _: Uuid = sqlx::query_scalar(
            "select n.uuid from fluxpro.process_def_node n \
            join fluxpro.process_instance i on i.process_def_uuid=n.process_def_uuid \
            where i.token=$1 and n.node_id=$2",
        )
        .bind(pending.task.process_token().get_id())
        .bind(target.get_id())
        .fetch_one(&mut *connection)
        .await?;
    }

    let uuid: Uuid = sqlx::query_scalar(
        "insert into fluxpro.queue_runner (task, run_after, node_visit_id) values ($1,$2,$3) returning uuid",
    )
    .bind(serde_json::to_value(&pending.task)?)
    .bind(pending.run_after.unwrap_or_else(Utc::now))
    .bind(pending.visit_id)
    .fetch_one(&mut *connection)
    .await?;
    if let FluxproQueueTask::ProcessEvent {
        process_token,
        node,
        ..
    } = &pending.task
    {
        for signal in &pending.cancel_signals {
            sqlx::query("insert into fluxpro.queue_cancel_task (process_token,node_id,event_id,task_uuid) values ($1,$2,$3,$4)")
                .bind(process_token.get_id()).bind(node.id().get_id()).bind(signal.get_id()).bind(uuid)
                .execute(&mut *connection).await?;
        }
    }
    Ok(())
}

async fn record_event(
    connection: &mut PgConnection,
    instance_uuid: Uuid,
    event: &ExecutionLogEvent,
) -> anyhow::Result<()> {
    sqlx::query("insert into fluxpro.process_instance_log \
        (process_instance_uuid,level,event_type,source,message,node_id,handler_id,queue_task_uuid,attempt,error_kind,error_message,details) \
        values ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
        .bind(instance_uuid).bind(event.level.to_string()).bind(&event.event_type).bind(&event.source)
        .bind(&event.message).bind(&event.node_id).bind(&event.handler_id).bind(event.queue_task_uuid)
        .bind(event.attempt).bind(&event.error_kind).bind(&event.error_message).bind(&event.details)
        .execute(connection).await?;
    Ok(())
}

pub(super) async fn commit(
    db: &FluxproDbServiceImpl,
    task: &FluxproQueueTaskDefinition,
    snapshot: &TaskExecutionSnapshot,
    changes: TaskExecutionChanges,
) -> anyhow::Result<()> {
    let mut tx = db.db_pool.begin().await?;
    let result: anyhow::Result<()> = async {
    let instance = sqlx::query(
        "select uuid, execution_state, process_def_uuid, revision, node_visit_id, wait_completed \
        from fluxpro.process_instance where token=$1 for update",
    )
    .bind(task.task.process_token().get_id())
    .fetch_one(&mut *tx)
    .await?;
    lock_task(&mut tx, task).await?;
    anyhow::ensure!(
        instance.try_get::<String, _>("execution_state")? == "running",
        "instance is suspended"
    );
    anyhow::ensure!(
        instance.try_get::<i64, _>("revision")? == snapshot.revision,
        "instance changed during task execution"
    );
    let instance_uuid: Uuid = instance.try_get("uuid")?;
    let definition_uuid: Uuid = instance.try_get("process_def_uuid")?;
    if changes.close_wait {
        anyhow::ensure!(
            snapshot.visit_id.is_some() && !instance.try_get::<bool, _>("wait_completed")?,
            "wait already completed"
        );
        anyhow::ensure!(
            instance.try_get::<Option<Uuid>, _>("node_visit_id")? == snapshot.visit_id,
            "wait visit changed"
        );
        // Remove sibling timeouts, but retain the source row until the final lease check.
        sqlx::query("delete from fluxpro.queue_cancel_task where task_uuid in \
            (select uuid from fluxpro.queue_runner where node_visit_id=$1 and task ? 'ProcessEvent' and uuid<>$2)")
            .bind(snapshot.visit_id).bind(task.uuid).execute(&mut *tx).await?;
        sqlx::query("delete from fluxpro.queue_runner where node_visit_id=$1 and task ? 'ProcessEvent' and uuid<>$2")
            .bind(snapshot.visit_id).bind(task.uuid).execute(&mut *tx).await?;
        sqlx::query("update fluxpro.process_instance set wait_completed=true where uuid=$1")
            .bind(instance_uuid)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(node_id) = &changes.enter_node {
        let node_uuid: Uuid = sqlx::query_scalar(
            "select uuid from fluxpro.process_def_node where process_def_uuid=$1 and node_id=$2",
        )
        .bind(definition_uuid)
        .bind(node_id.get_id())
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("update fluxpro.process_instance set current_node_ref=$2, node_visit_id=$3, wait_completed=false where uuid=$1")
            .bind(instance_uuid).bind(node_uuid).bind(task.uuid).execute(&mut *tx).await?;
    }
    if changes.context_patch.is_none()
        && let Some(context) = &changes.context
    {
        merge_context(&mut tx, instance_uuid, context).await?;
    }
    if let Some(patch) = &changes.context_patch {
        apply_patch(&mut tx, instance_uuid, patch).await?;
    }

    if let Some(stage) = &changes.stage {
        assign_stage(&mut tx, instance_uuid, definition_uuid, stage).await?;
    }
    for pending in &changes.tasks {
        anyhow::ensure!(
            pending.task.process_token() == task.task.process_token(),
            "successor belongs to another instance"
        );
        enqueue(&mut tx, pending).await?;
    }
    for event in &changes.events {
        record_event(&mut tx, instance_uuid, event).await?;
    }
    sqlx::query("update fluxpro.process_instance set revision=revision+1 where uuid=$1")
        .bind(instance_uuid)
        .execute(&mut *tx)
        .await?;
    if let Some((kind, reason)) = &changes.incident {
        anyhow::ensure!(
            changes.tasks.is_empty() && changes.retry_at.is_none(),
            "incident cannot also schedule successors or retries"
        );
        sqlx::query("insert into fluxpro.process_incident(process_instance_uuid,task_uuid,task,kind,reason,attempt) values ($1,$2,$3,$4,$5,$6)")
            .bind(instance_uuid).bind(task.uuid).bind(serde_json::to_value(&task.task)?).bind(kind).bind(reason).bind(task.attempts)
            .execute(&mut *tx).await?;
        sqlx::query(
            "update fluxpro.process_instance set execution_state='suspended' where uuid=$1",
        )
        .bind(instance_uuid)
        .execute(&mut *tx)
        .await?;
    } else if changes.retry_at.is_none() {
        sqlx::query("update fluxpro.process_instance set recovery_task_uuid=null where uuid=$1 and recovery_task_uuid=$2")
            .bind(instance_uuid).bind(task.uuid).execute(&mut *tx).await?;
    }
    // clock_timestamp checks actual elapsed time, not transaction-start time.
    let affected = if let Some(retry_at) = changes
        .retry_at
        .or_else(|| changes.incident.as_ref().map(|_| Utc::now()))
    {
        sqlx::query("update fluxpro.queue_runner set run_after=$3,lock_key='',locked_at=null,locked_by=null \
            where uuid=$1 and lock_key=$2 and locked_by>clock_timestamp()")
            .bind(task.uuid).bind(&task.lock_key).bind(retry_at).execute(&mut *tx).await?.rows_affected()
    } else {
        sqlx::query("delete from fluxpro.queue_cancel_task where task_uuid=$1")
            .bind(task.uuid)
            .execute(&mut *tx)
            .await?;
        sqlx::query("delete from fluxpro.queue_runner where uuid=$1 and lock_key=$2 and locked_by>clock_timestamp()")
            .bind(task.uuid).bind(&task.lock_key).execute(&mut *tx).await?.rows_affected()
    };
    anyhow::ensure!(affected == 1, "task lease expired before transition commit");
        Ok(())
    }
    .await;
    if let Err(error) = result {
        // Drop only queues SQLx rollback. Release all row locks before a caller
        // retries or another worker tries to reclaim this task with SKIP LOCKED.
        tx.rollback().await?;
        return Err(error);
    }
    tx.commit().await?;
    Ok(())
}

pub(super) async fn start(
    db: &FluxproDbServiceImpl,
    definition: &ProcessDefinition,
    command: StartProcessInstance,
) -> anyhow::Result<IdField> {
    let definition_uuid = definition
        .uuid
        .ok_or_else(|| anyhow::anyhow!("definition has no database identity"))?;
    let start = definition
        .nodes
        .iter()
        .find(|node| node.is_start())
        .ok_or_else(|| anyhow::anyhow!("Start node missing"))?;
    let initial = definition
        .stages
        .iter()
        .find(|stage| stage.is_initial())
        .ok_or_else(|| anyhow::anyhow!("initial stage missing"))?;
    let token = IdField::generate();
    let mut tx = db.db_pool.begin().await?;
    // Publication may have changed since version lookup. Lock it through startup.
    sqlx::query("select uuid from fluxpro.process_definition where uuid=$1 and lower(status)='active' and effective_from<=clock_timestamp() and (deprecated_at is null or deprecated_at>clock_timestamp()) for share")
        .bind(definition_uuid).fetch_one(&mut *tx).await?;
    let instance_uuid: Uuid = sqlx::query_scalar("insert into fluxpro.process_instance (process_def_uuid,process_id,token) values ($1,$2,$3) returning uuid")
        .bind(definition_uuid).bind(command.process_id.get_id()).bind(token.get_id()).fetch_one(&mut *tx).await?;
    merge_context(&mut tx, instance_uuid, &command.context).await?;
    assign_stage(
        &mut tx,
        instance_uuid,
        definition_uuid,
        &SetStageWithReason {
            stage: initial.id().clone(),
            reason: None,
        },
    )
    .await?;
    enqueue(
        &mut tx,
        &ScheduledTask {
            task: FluxproQueueTask::ProcessNode {
                process_token: token.clone(),
                node: start.clone(),
            },
            run_after: None,
            visit_id: None,
            cancel_signals: vec![],
        },
    )
    .await?;
    record_event(
        &mut tx,
        instance_uuid,
        &ExecutionLogEvent::info(
            "instance.created",
            "process_service",
            "Process instance created",
        ),
    )
    .await?;
    record_event(
        &mut tx,
        instance_uuid,
        &ExecutionLogEvent::info(
            "stage.changed",
            "process_service",
            format!("Process moved to stage {}", initial.id()),
        ),
    )
    .await?;
    tx.commit().await?;
    Ok(token)
}
