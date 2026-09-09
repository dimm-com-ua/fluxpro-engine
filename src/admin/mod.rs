//! Paginated inspection and lease-aware queue maintenance for administration UIs.

use crate::models::context_map::context_map::{ContextMap, ContextValue};
use crate::models::id_field::IdField;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgPool, Row};
use thiserror::Error;
use uuid::Uuid;

mod audit;
mod export;

pub use audit::*;

const DEFAULT_PAGE_SIZE: i64 = 50;
const MAX_PAGE_SIZE: i64 = 250;

/// Administration lookup, context decoding, or database failure.
#[derive(Debug, Error)]
pub enum AdminError {
    /// The requested administrative resource does not exist.
    #[error("requested Fluxpro resource was not found")]
    NotFound,
    /// A stored context key is not a valid identifier.
    #[error("invalid context key '{0}'")]
    InvalidContextKey(String),
    /// An administration database operation failed.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Result type returned by administration queries and queue maintenance.
pub type AdminResult<T> = Result<T, AdminError>;

/// Pagination request; offset is clamped to zero and limit to 1–250.
///
/// The default page starts at zero and contains up to 50 items.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PageRequest {
    /// Number of matching rows to skip; requests are clamped to zero.
    pub offset: i64,
    /// Maximum rows in a page; administration requests are clamped to 1–250.
    pub limit: i64,
}

impl Default for PageRequest {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: DEFAULT_PAGE_SIZE,
        }
    }
}

impl PageRequest {
    fn normalized(self) -> Self {
        Self {
            offset: self.offset.max(0),
            limit: self.limit.clamp(1, MAX_PAGE_SIZE),
        }
    }
}

/// A page of results with total count and normalized pagination parameters.
#[derive(Debug, Clone, Serialize)]
pub struct Page<T> {
    /// Rows in this page.
    pub items: Vec<T>,
    /// Total number of matching rows before pagination.
    pub total: i64,
    /// Number of matching rows to skip; requests are clamped to zero.
    pub offset: i64,
    /// Maximum rows in a page; administration requests are clamped to 1–250.
    pub limit: i64,
}

/// Optional text and status filters for stored process definitions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessDefinitionFilter {
    /// Optional case-insensitive text search; blank input disables the filter.
    pub search: Option<String>,
    /// Stored lifecycle status of the definition version.
    pub status: Option<String>,
}

/// Stored version metadata without the full workflow payload.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ProcessDefinitionSummary {
    /// Database row identity.
    pub uuid: Uuid,
    /// UTC time when the row was created.
    pub created_at: DateTime<Utc>,
    /// Logical process definition key shared across versions.
    pub key: String,
    /// Normalized process definition version.
    pub version: String,
    /// Insertion-order index for versions of the same process key.
    pub index_id: i32,
    /// Stored lifecycle status of the definition version.
    pub status: String,
    /// UTC time from which this definition can be selected.
    pub effective_from: DateTime<Utc>,
    /// UTC time after which this definition is excluded from runtime selection.
    pub deprecated_at: Option<DateTime<Utc>>,
}

/// Stored version metadata, compiled JSON, and optional original source.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ProcessDefinitionDetails {
    /// Database row identity.
    pub uuid: Uuid,
    /// UTC time when the row was created.
    pub created_at: DateTime<Utc>,
    /// Logical process definition key shared across versions.
    pub key: String,
    /// Normalized process definition version.
    pub version: String,
    /// Insertion-order index for versions of the same process key.
    pub index_id: i32,
    /// Stored lifecycle status of the definition version.
    pub status: String,
    /// UTC time from which this definition can be selected.
    pub effective_from: DateTime<Utc>,
    /// UTC time after which this definition is excluded from runtime selection.
    pub deprecated_at: Option<DateTime<Utc>>,
    /// Human-readable notes supplied by the version metadata.
    pub version_comment: Option<String>,
    /// Compiled workflow definition as JSON.
    pub definition: Value,
    /// Original source text when provided during definition creation.
    pub source_definition: Option<String>,
}

/// Number of instances currently pointing at a definition node.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ProcessNodeInstanceCount {
    /// Workflow-local node identifier.
    pub node_id: String,
    /// Number of instances currently assigned to this node.
    pub instance_count: i64,
}

/// Aggregated version and instance information for one process key.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ProcessOverview {
    /// Logical process definition key shared across versions.
    pub key: String,
    /// Display name of the selected process definition.
    pub name: String,
    /// Number of stored versions for this process key.
    pub versions_count: i64,
    /// Version selected by the overview query for this process key.
    pub current_version: String,
    /// Stored lifecycle status of the selected version.
    pub current_status: String,
    /// Effective-from timestamp of the selected version.
    pub current_effective_from: DateTime<Utc>,
    /// Optional deprecation timestamp of the selected version.
    pub current_deprecated_at: Option<DateTime<Utc>>,
    /// Optional team or person responsible for the definition.
    pub owner: Option<String>,
    /// Descriptive ISO 8601 service-level target; not enforced by the runtime.
    pub sla: Option<String>,
    /// Human-readable notes supplied by the version metadata.
    pub version_comment: Option<String>,
    /// Number of instances bound to the selected version.
    pub current_instance_count: i64,
    /// Number of instances across all versions of this process key.
    pub total_instance_count: i64,
}

/// Optional search, definition, and derived-state filters for instances.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessInstanceFilter {
    /// Optional case-insensitive text search; blank input disables the filter.
    pub search: Option<String>,
    /// Database identity of the bound process definition.
    pub process_definition_uuid: Option<Uuid>,
    /// `created`, `running`, or `completed`.
    pub state: Option<String>,
}

/// Instance identity and its current node and business stage.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ProcessInstanceSummary {
    /// Database row identity.
    pub uuid: Uuid,
    /// UTC time when the row was created.
    pub created_at: DateTime<Utc>,
    /// Database identity of the bound process definition.
    pub process_definition_uuid: Uuid,
    /// Logical key shared by versions of the same process.
    pub process_key: String,
    /// Version bound to this instance.
    pub process_version: String,
    /// Application business identifier; distinct from the generated runtime token.
    pub process_id: String,
    /// Generated runtime token used for signals and service operations.
    pub token: String,
    /// Derived state exposed by the administration query.
    pub state: String,
    /// Database identity of the currently assigned node.
    pub current_node_uuid: Option<Uuid>,
    /// Workflow-local identifier of the currently assigned node.
    pub current_node_id: Option<String>,
    /// Serialized type of the currently assigned node.
    pub current_node_type: Option<String>,
    /// Current node visit identity; include it in signals that target this wait.
    pub node_visit_id: Option<Uuid>,
    /// True once this visit has accepted its closing signal or timeout.
    pub wait_completed: bool,
    /// Database identity of the current business stage.
    pub current_stage_uuid: Option<Uuid>,
    /// Definition-local identifier of the current stage.
    pub current_stage_id: Option<String>,
    /// Display name of the current business stage.
    pub current_stage_name: Option<String>,
    /// Persisted explanation for the current stage, when available.
    pub current_stage_reason: Option<String>,
}

/// Persisted stage transition with its timestamp and context snapshot.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ProcessStageHistoryEntry {
    /// Database row identity.
    pub uuid: Uuid,
    /// Database identity of the stage recorded by this history entry.
    pub stage_uuid: Option<Uuid>,
    /// Definition-local identifier of the recorded stage.
    pub stage_id: Option<String>,
    /// Display name of the recorded stage.
    pub stage_name: Option<String>,
    /// Optional explanation associated with this stage assignment.
    pub reason: Option<String>,
    /// UTC time when the row was created.
    pub created_at: DateTime<Utc>,
    /// JSON context snapshot recorded with the stage transition.
    pub context: Value,
}

/// Instance summary, current node, typed context, and stage history.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessInstanceDetails {
    /// Instance identity and its current node and stage metadata.
    #[serde(flatten)]
    pub summary: ProcessInstanceSummary,
    /// Compiled JSON of the currently assigned node.
    pub current_node: Option<Value>,
    /// Current typed context flattened by variable name.
    pub context: ContextMap,
    /// Stored context variables including their database IDs and scopes.
    pub context_variables: Vec<ProcessContextVariable>,
    /// Recorded stage transitions for this instance.
    pub stage_history: Vec<ProcessStageHistoryEntry>,
}

/// A scoped variable with its database identity and typed value.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessContextVariable {
    /// Database row identity.
    pub uuid: Uuid,
    /// Variable namespace; runtime writes normally use `_`.
    pub scope: String,
    /// Context variable name within its stored scope.
    pub name: IdField,
    /// Typed value of the stored context variable.
    pub value: ContextValue,
}

/// Optional severity and event-type filters for execution history.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionLogFilter {
    /// Execution event severity.
    pub level: Option<String>,
    /// Stable event category, such as `node.entered`.
    pub event_type: Option<String>,
}

/// Stored execution event with diagnostic and queue-attempt metadata.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ExecutionLogEntry {
    /// Database row identity.
    pub uuid: Uuid,
    /// UTC time when the row was created.
    pub created_at: DateTime<Utc>,
    /// Execution event severity.
    pub level: String,
    /// Stable event category, such as `node.entered`.
    pub event_type: String,
    /// Component that emitted the event.
    pub source: String,
    /// Human-readable event description.
    pub message: String,
    /// Workflow-local node identifier.
    pub node_id: Option<String>,
    /// Registered handler identifier, when applicable.
    pub handler_id: Option<String>,
    /// Database identity of the associated queue item.
    pub queue_task_uuid: Option<Uuid>,
    /// Queue claim count associated with this event.
    pub attempt: Option<i32>,
    /// Machine-readable error category, when available.
    pub error_kind: Option<String>,
    /// Error description, when available.
    pub error_message: Option<String>,
    /// Additional diagnostic details for this event or issue.
    pub details: Value,
}

/// Accepted signal submission recorded before queued delivery.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct SignalHistoryEntry {
    /// Database row identity.
    pub uuid: Uuid,
    /// UTC time when the row was created.
    pub created_at: DateTime<Utc>,
    /// Runtime instance token stored under the legacy `process_id` column name.
    pub process_id: String,
    /// Name of the admitted signal.
    pub signal_name: String,
    /// Signal context payload recorded at admission time.
    pub payload: Value,
}

/// Optional filter for a derived queue availability state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueueTaskFilter {
    /// `ready`, `scheduled`, or `leased`.
    pub state: Option<String>,
}

/// Administrative view of a task, its payload, and its lease.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct QueueTaskSummary {
    /// Database row identity.
    pub uuid: Uuid,
    /// UTC time when the row was created.
    pub created_at: DateTime<Utc>,
    /// Earliest UTC time at which the task may be claimed.
    pub run_after: DateTime<Utc>,
    /// Number of queue claims, including the current attempt.
    pub attempts: i32,
    /// Derived state exposed by the administration query.
    pub state: String,
    /// Lease ownership key; an empty string indicates an unleased task.
    pub lock_key: String,
    /// UTC time at which the current lease was acquired.
    pub locked_at: Option<DateTime<Utc>>,
    /// UTC time at which the current task lease expires.
    pub lease_expires_at: Option<DateTime<Utc>>,
    /// Serialized or decoded queue work payload.
    pub task: Value,
}

/// Framework-independent administration queries and lease-aware queue operations.
#[derive(Clone)]
pub struct FluxproAdminService {
    pool: PgPool,
}

impl FluxproAdminService {
    /// Uses an existing pool for administration queries.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Reads the unresolved durable incident for an instance.
    pub async fn get_open_incident(
        &self,
        token: &IdField,
    ) -> anyhow::Result<Option<crate::db_service::incidents::ProcessIncident>> {
        crate::db_service::FluxproDbServiceImpl::new(self.pool.clone())
            .get_open_incident(token)
            .await
    }

    /// Explicitly resumes the specified incident; duplicate or stale commands return false.
    pub async fn resume_instance(
        &self,
        token: &IdField,
        incident_id: Uuid,
    ) -> anyhow::Result<bool> {
        crate::db_service::FluxproDbServiceImpl::new(self.pool.clone())
            .resume_instance(token, incident_id)
            .await
    }

    /// Returns a filtered page of stored definition versions.
    pub async fn list_process_definitions(
        &self,
        filter: ProcessDefinitionFilter,
        page: PageRequest,
    ) -> AdminResult<Page<ProcessDefinitionSummary>> {
        let page = page.normalized();
        let search = normalize_filter(filter.search);
        let status = normalize_filter(filter.status);
        let total = sqlx::query_scalar::<_, i64>(
            r#"select count(*)
               from fluxpro.process_definition
               where ($1::text is null or key_ ilike '%' || $1 || '%' or version ilike '%' || $1 || '%')
                 and ($2::text is null or status = $2)"#,
        )
        .bind(&search)
        .bind(&status)
        .fetch_one(&self.pool)
        .await?;
        let items = sqlx::query_as::<_, ProcessDefinitionSummary>(
            r#"select uuid, created_at, key_ as key, version, index_id, status,
                      effective_from, deprecated_at
               from fluxpro.process_definition
               where ($1::text is null or key_ ilike '%' || $1 || '%' or version ilike '%' || $1 || '%')
                 and ($2::text is null or status = $2)
               order by created_at desc, uuid
               offset $3 limit $4"#,
        )
        .bind(&search)
        .bind(&status)
        .bind(page.offset)
        .bind(page.limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(Page {
            items,
            total,
            offset: page.offset,
            limit: page.limit,
        })
    }

    /// Loads one stored version, including compiled JSON and original source.
    pub async fn get_process_definition(
        &self,
        uuid: Uuid,
    ) -> AdminResult<ProcessDefinitionDetails> {
        sqlx::query_as(
            r#"select uuid, created_at, key_ as key, version, index_id, status,
                      effective_from, deprecated_at, version_comment, definition, source_definition
               from fluxpro.process_definition where uuid = $1"#,
        )
        .bind(uuid)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AdminError::NotFound)
    }

    /// Loads a version by key, prioritizing active status and then insertion index.
    ///
    /// Unlike runtime selection, this administration lookup does not filter dates.
    pub async fn get_current_process_definition_by_key(
        &self,
        key: &str,
    ) -> AdminResult<ProcessDefinitionDetails> {
        sqlx::query_as(
            r#"select uuid, created_at, key_ as key, version, index_id, status,
                      effective_from, deprecated_at, version_comment, definition, source_definition
               from fluxpro.process_definition
               where key_ = $1
               order by
                   case when status = 'Active' or lower(status) = 'active' then 0 else 1 end,
                   index_id desc
               limit 1"#,
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AdminError::NotFound)
    }

    /// Counts instances currently assigned to each node of a definition.
    pub async fn get_process_node_instance_counts(
        &self,
        process_definition_uuid: Uuid,
    ) -> AdminResult<Vec<ProcessNodeInstanceCount>> {
        Ok(sqlx::query_as(
            r#"select n.node_id, count(i.uuid)::bigint as instance_count
               from fluxpro.process_def_node n
               left join fluxpro.process_instance i on i.current_node_ref = n.uuid
               where n.process_def_uuid = $1
               group by n.uuid, n.node_id
               order by n.node_id"#,
        )
        .bind(process_definition_uuid)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Returns per-key summaries with version and instance counts.
    pub async fn list_process_overviews(
        &self,
        search: Option<String>,
        page: PageRequest,
    ) -> AdminResult<Page<ProcessOverview>> {
        let page = page.normalized();
        let search = normalize_filter(search);
        let total = sqlx::query_scalar::<_, i64>(
            r#"select count(distinct key_)
               from fluxpro.process_definition
               where $1::text is null or key_ ilike '%' || $1 || '%'"#,
        )
        .bind(&search)
        .fetch_one(&self.pool)
        .await?;
        let items = sqlx::query_as::<_, ProcessOverview>(PROCESS_OVERVIEW_SELECT)
            .bind(&search)
            .bind(page.offset)
            .bind(page.limit)
            .fetch_all(&self.pool)
            .await?;
        Ok(Page {
            items,
            total,
            offset: page.offset,
            limit: page.limit,
        })
    }

    /// Returns filtered instances with derived state, node, and stage information.
    pub async fn list_process_instances(
        &self,
        filter: ProcessInstanceFilter,
        page: PageRequest,
    ) -> AdminResult<Page<ProcessInstanceSummary>> {
        let page = page.normalized();
        let search = normalize_filter(filter.search);
        let state = normalize_filter(filter.state);
        let total = sqlx::query_scalar::<_, i64>(&format!(
            "select count(*) from ({INSTANCE_SELECT}) instance_admin \
             where ($1::text is null or process_id ilike '%' || $1 || '%' \
                    or token ilike '%' || $1 || '%' or process_key ilike '%' || $1 || '%') \
               and ($2::uuid is null or process_definition_uuid = $2) \
               and ($3::text is null or state = $3)"
        ))
        .bind(&search)
        .bind(filter.process_definition_uuid)
        .bind(&state)
        .fetch_one(&self.pool)
        .await?;
        let items = sqlx::query_as::<_, ProcessInstanceSummary>(&format!(
            "select * from ({INSTANCE_SELECT}) instance_admin \
             where ($1::text is null or process_id ilike '%' || $1 || '%' \
                    or token ilike '%' || $1 || '%' or process_key ilike '%' || $1 || '%') \
               and ($2::uuid is null or process_definition_uuid = $2) \
               and ($3::text is null or state = $3) \
             order by created_at desc, uuid offset $4 limit $5"
        ))
        .bind(&search)
        .bind(filter.process_definition_uuid)
        .bind(&state)
        .bind(page.offset)
        .bind(page.limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(Page {
            items,
            total,
            offset: page.offset,
            limit: page.limit,
        })
    }

    /// Loads an instance with its context, current node, and stage history.
    pub async fn get_process_instance(&self, uuid: Uuid) -> AdminResult<ProcessInstanceDetails> {
        let summary = sqlx::query_as::<_, ProcessInstanceSummary>(&format!(
            "select * from ({INSTANCE_SELECT}) instance_admin where uuid = $1"
        ))
        .bind(uuid)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(AdminError::NotFound)?;

        let current_node = sqlx::query_scalar::<_, Value>(
            r#"select n.definition
               from fluxpro.process_instance i
               join fluxpro.process_def_node n on n.uuid = i.current_node_ref
               where i.uuid = $1"#,
        )
        .bind(uuid)
        .fetch_optional(&self.pool)
        .await?;

        let mut context = ContextMap::default();
        let mut context_variables = Vec::new();
        for row in sqlx::query(
            r#"select uuid, scope, name, value
               from fluxpro.process_instance_context_variable
               where process_instance_uuid = $1 order by scope, name"#,
        )
        .bind(uuid)
        .fetch_all(&self.pool)
        .await?
        {
            let name: String = row.try_get("name")?;
            let key = IdField::new(&name).map_err(|_| AdminError::InvalidContextKey(name))?;
            let value: Value = row.try_get("value")?;
            let value = serde_json::from_value::<ContextValue>(value)
                .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
            context.0.insert(key.clone(), value.clone());
            context_variables.push(ProcessContextVariable {
                uuid: row.try_get("uuid")?,
                scope: row.try_get("scope")?,
                name: key,
                value,
            });
        }

        let stage_history = sqlx::query_as(
            r#"select l.uuid, l.stage_uuid, s.stage_id, s.name as stage_name,
                      l.reason, l.created_at, l.context
               from fluxpro.process_instance_stage_log l
               left join fluxpro.process_stage s on s.uuid = l.stage_uuid
               where l.process_instance_uuid = $1
               order by l.created_at desc, l.uuid"#,
        )
        .bind(uuid)
        .fetch_all(&self.pool)
        .await?;

        Ok(ProcessInstanceDetails {
            summary,
            current_node,
            context,
            context_variables,
            stage_history,
        })
    }

    /// Returns a filtered page of execution events for an instance.
    pub async fn list_process_instance_logs(
        &self,
        process_instance_uuid: Uuid,
        filter: ExecutionLogFilter,
        page: PageRequest,
    ) -> AdminResult<Page<ExecutionLogEntry>> {
        let page = page.normalized();
        let level = normalize_filter(filter.level);
        let event_type = normalize_filter(filter.event_type);
        let total = sqlx::query_scalar::<_, i64>(
            r#"select count(*) from fluxpro.process_instance_log
               where process_instance_uuid = $1
                 and ($2::text is null or level = $2)
                 and ($3::text is null or event_type = $3)"#,
        )
        .bind(process_instance_uuid)
        .bind(&level)
        .bind(&event_type)
        .fetch_one(&self.pool)
        .await?;
        let items = sqlx::query_as::<_, ExecutionLogEntry>(
            r#"select uuid, created_at, level, event_type, source, message, node_id,
                      handler_id, queue_task_uuid, attempt, error_kind, error_message, details
               from fluxpro.process_instance_log
               where process_instance_uuid = $1
                 and ($2::text is null or level = $2)
                 and ($3::text is null or event_type = $3)
               order by created_at desc, uuid desc offset $4 limit $5"#,
        )
        .bind(process_instance_uuid)
        .bind(&level)
        .bind(&event_type)
        .bind(page.offset)
        .bind(page.limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(Page {
            items,
            total,
            offset: page.offset,
            limit: page.limit,
        })
    }

    /// Returns a page of admitted signals and their typed payloads.
    pub async fn list_process_instance_signals(
        &self,
        process_instance_uuid: Uuid,
        page: PageRequest,
    ) -> AdminResult<Page<SignalHistoryEntry>> {
        let page = page.normalized();
        let total = sqlx::query_scalar::<_, i64>(
            r#"select count(*)
               from fluxpro.signal_history h
               join fluxpro.process_instance i on i.token = h.process_id
               where i.uuid = $1"#,
        )
        .bind(process_instance_uuid)
        .fetch_one(&self.pool)
        .await?;
        let items = sqlx::query_as::<_, SignalHistoryEntry>(
            r#"select h.uuid, h.created_at, h.process_id, h.signal_name, h.payload
               from fluxpro.signal_history h
               join fluxpro.process_instance i on i.token = h.process_id
               where i.uuid = $1
               order by h.created_at desc, h.uuid desc
               offset $2 limit $3"#,
        )
        .bind(process_instance_uuid)
        .bind(page.offset)
        .bind(page.limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(Page {
            items,
            total,
            offset: page.offset,
            limit: page.limit,
        })
    }

    /// Returns a filtered page of ready, scheduled, or leased tasks across instances.
    pub async fn list_queue_tasks(
        &self,
        filter: QueueTaskFilter,
        page: PageRequest,
    ) -> AdminResult<Page<QueueTaskSummary>> {
        let page = page.normalized();
        let state = normalize_filter(filter.state);
        let total = sqlx::query_scalar::<_, i64>(&format!(
            "select count(*) from ({QUEUE_SELECT}) queue_admin where ($1::text is null or state = $1)"
        ))
        .bind(&state)
        .fetch_one(&self.pool)
        .await?;
        let items = sqlx::query_as::<_, QueueTaskSummary>(&format!(
            "select * from ({QUEUE_SELECT}) queue_admin \
             where ($1::text is null or state = $1) \
             order by run_after, created_at offset $2 limit $3"
        ))
        .bind(&state)
        .bind(page.offset)
        .bind(page.limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(Page {
            items,
            total,
            offset: page.offset,
            limit: page.limit,
        })
    }

    /// Makes an unlocked or expired task immediately available for retry.
    pub async fn retry_queue_task(&self, uuid: Uuid) -> AdminResult<bool> {
        let result = sqlx::query(
            r#"update fluxpro.queue_runner
               set run_after = now(), lock_key = '', locked_at = null, locked_by = null
               where uuid = $1 and (lock_key = '' or locked_by is null or locked_by < now())"#,
        )
        .bind(uuid)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Cancels an unlocked or expired task and its cancellation metadata.
    pub async fn cancel_queue_task(&self, uuid: Uuid) -> AdminResult<bool> {
        let mut transaction = self.pool.begin().await?;
        let task = sqlx::query_scalar::<_, Uuid>(
            r#"select uuid from fluxpro.queue_runner
               where uuid = $1 and (lock_key = '' or locked_by is null or locked_by < now())
               and not exists (select 1 from fluxpro.process_incident where task_uuid=$1 and resolved_at is null)
               and not exists (select 1 from fluxpro.process_instance where recovery_task_uuid=$1)
               for update"#,
        )
        .bind(uuid)
        .fetch_optional(&mut *transaction)
        .await?;
        if task.is_some() {
            sqlx::query("delete from fluxpro.queue_cancel_task where task_uuid = $1")
                .bind(uuid)
                .execute(&mut *transaction)
                .await?;
            sqlx::query("delete from fluxpro.queue_runner where uuid = $1")
                .bind(uuid)
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
            Ok(true)
        } else {
            transaction.rollback().await?;
            Ok(false)
        }
    }
}

fn normalize_filter(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_owned();
        (!value.is_empty()).then_some(value)
    })
}

const INSTANCE_SELECT: &str = r#"
    select i.uuid,
           i.created_at,
           i.process_def_uuid as process_definition_uuid,
           d.key_ as process_key,
           d.version as process_version,
           i.process_id,
           i.token,
           case
               when i.execution_state = 'suspended' then 'suspended'
               when i.current_node_ref is null then 'created'
               when lower(coalesce(n.definition->>'type', '')) = 'end' then 'completed'
               else 'running'
           end as state,
           i.current_node_ref as current_node_uuid,
           n.node_id as current_node_id,
           n.definition->>'type' as current_node_type,
           i.node_visit_id,
           i.wait_completed,
           i.current_stage as current_stage_uuid,
           s.stage_id as current_stage_id,
           s.name as current_stage_name,
           i.current_stage_reason
    from fluxpro.process_instance i
    join fluxpro.process_definition d on d.uuid = i.process_def_uuid
    left join fluxpro.process_def_node n on n.uuid = i.current_node_ref
    left join fluxpro.process_stage s on s.uuid = i.current_stage
"#;

const QUEUE_SELECT: &str = r#"
    select uuid,
           created_at,
           run_after,
           attempts,
           case
               when exists (select 1 from fluxpro.process_instance i where
                   i.token=coalesce(task #>> '{process_node,process_token}',task #>> '{ProcessEvent,process_token}',task #>> '{ProcessSignal,process_token}')
                   and i.execution_state='suspended') then 'suspended'
               when lock_key <> '' and locked_by >= now() then 'leased'
               when run_after > now() then 'scheduled'
               else 'ready'
           end as state,
           lock_key,
           locked_at,
           locked_by as lease_expires_at,
           task
    from fluxpro.queue_runner
"#;

const PROCESS_OVERVIEW_SELECT: &str = r#"
    with instance_counts as (
        select process_def_uuid, count(*)::bigint as instance_count
        from fluxpro.process_instance
        group by process_def_uuid
    ),
    ranked_definitions as (
        select d.*,
               coalesce(ic.instance_count, 0)::bigint as instance_count,
               row_number() over (
                   partition by d.key_
                   order by (lower(d.status) = 'active') desc,
                            d.index_id desc,
                            d.created_at desc
               ) as version_rank
        from fluxpro.process_definition d
        left join instance_counts ic on ic.process_def_uuid = d.uuid
    ),
    definition_stats as (
        select key_,
               count(*)::bigint as versions_count,
               sum(instance_count)::bigint as total_instance_count
        from ranked_definitions
        group by key_
    )
    select current.key_ as key,
           coalesce(nullif(current.definition->>'name', ''), current.key_) as name,
           stats.versions_count,
           current.version as current_version,
           current.status as current_status,
           current.effective_from as current_effective_from,
           current.deprecated_at as current_deprecated_at,
           current.definition #>> '{metadata,owner}' as owner,
           current.definition #>> '{metadata,sla}' as sla,
           coalesce(current.version_comment, current.definition #>> '{metadata,comment}') as version_comment,
           current.instance_count as current_instance_count,
           stats.total_instance_count
    from ranked_definitions current
    join definition_stats stats on stats.key_ = current.key_
    where current.version_rank = 1
      and ($1::text is null or current.key_ ilike '%' || $1 || '%'
           or current.definition->>'name' ilike '%' || $1 || '%')
    order by current.key_
    offset $2 limit $3
"#;
