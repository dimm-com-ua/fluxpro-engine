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

#[derive(Debug, Error)]
pub enum AdminError {
    #[error("requested Fluxpro resource was not found")]
    NotFound,
    #[error("invalid context key '{0}'")]
    InvalidContextKey(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

pub type AdminResult<T> = Result<T, AdminError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PageRequest {
    pub offset: i64,
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

#[derive(Debug, Clone, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessDefinitionFilter {
    pub search: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ProcessDefinitionSummary {
    pub uuid: Uuid,
    pub created_at: DateTime<Utc>,
    pub key: String,
    pub version: String,
    pub index_id: i32,
    pub status: String,
    pub effective_from: DateTime<Utc>,
    pub deprecated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ProcessDefinitionDetails {
    pub uuid: Uuid,
    pub created_at: DateTime<Utc>,
    pub key: String,
    pub version: String,
    pub index_id: i32,
    pub status: String,
    pub effective_from: DateTime<Utc>,
    pub deprecated_at: Option<DateTime<Utc>>,
    pub version_comment: Option<String>,
    pub definition: Value,
    pub source_definition: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ProcessNodeInstanceCount {
    pub node_id: String,
    pub instance_count: i64,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ProcessOverview {
    pub key: String,
    pub name: String,
    pub versions_count: i64,
    pub current_version: String,
    pub current_status: String,
    pub current_effective_from: DateTime<Utc>,
    pub current_deprecated_at: Option<DateTime<Utc>>,
    pub owner: Option<String>,
    pub sla: Option<String>,
    pub version_comment: Option<String>,
    pub current_instance_count: i64,
    pub total_instance_count: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessInstanceFilter {
    pub search: Option<String>,
    pub process_definition_uuid: Option<Uuid>,
    /// `created`, `running`, or `completed`.
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ProcessInstanceSummary {
    pub uuid: Uuid,
    pub created_at: DateTime<Utc>,
    pub process_definition_uuid: Uuid,
    pub process_key: String,
    pub process_version: String,
    pub process_id: String,
    pub token: String,
    pub state: String,
    pub current_node_uuid: Option<Uuid>,
    pub current_node_id: Option<String>,
    pub current_node_type: Option<String>,
    pub current_stage_uuid: Option<Uuid>,
    pub current_stage_id: Option<String>,
    pub current_stage_name: Option<String>,
    pub current_stage_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ProcessStageHistoryEntry {
    pub uuid: Uuid,
    pub stage_uuid: Option<Uuid>,
    pub stage_id: Option<String>,
    pub stage_name: Option<String>,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub context: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessInstanceDetails {
    #[serde(flatten)]
    pub summary: ProcessInstanceSummary,
    pub current_node: Option<Value>,
    pub context: ContextMap,
    pub context_variables: Vec<ProcessContextVariable>,
    pub stage_history: Vec<ProcessStageHistoryEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessContextVariable {
    pub uuid: Uuid,
    pub scope: String,
    pub name: IdField,
    pub value: ContextValue,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionLogFilter {
    pub level: Option<String>,
    pub event_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ExecutionLogEntry {
    pub uuid: Uuid,
    pub created_at: DateTime<Utc>,
    pub level: String,
    pub event_type: String,
    pub source: String,
    pub message: String,
    pub node_id: Option<String>,
    pub handler_id: Option<String>,
    pub queue_task_uuid: Option<Uuid>,
    pub attempt: Option<i32>,
    pub error_kind: Option<String>,
    pub error_message: Option<String>,
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct SignalHistoryEntry {
    pub uuid: Uuid,
    pub created_at: DateTime<Utc>,
    pub process_id: String,
    pub signal_name: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueueTaskFilter {
    /// `ready`, `scheduled`, or `leased`.
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct QueueTaskSummary {
    pub uuid: Uuid,
    pub created_at: DateTime<Utc>,
    pub run_after: DateTime<Utc>,
    pub attempts: i32,
    pub state: String,
    pub lock_key: String,
    pub locked_at: Option<DateTime<Utc>>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub task: Value,
}

#[derive(Clone)]
pub struct FluxproAdminService {
    pool: PgPool,
}

impl FluxproAdminService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

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
               when i.current_node_ref is null then 'created'
               when lower(coalesce(n.definition->>'type', '')) = 'end' then 'completed'
               else 'running'
           end as state,
           i.current_node_ref as current_node_uuid,
           n.node_id as current_node_id,
           n.definition->>'type' as current_node_type,
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
