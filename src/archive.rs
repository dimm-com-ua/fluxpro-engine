use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FluxproArchiveSource {
    ExecutionLog,
    SignalHistory,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, FromRow)]
pub struct FluxproArchiveRecord {
    pub uuid: Uuid,
    pub event_at: DateTime<Utc>,
    pub data: Value,
}

#[derive(Clone)]
pub struct FluxproArchiveService {
    pool: PgPool,
}

impl FluxproArchiveService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Returns an ordered batch from engine-owned technical log tables.
    /// Host applications must use this API instead of querying FluxPro tables directly.
    pub async fn fetch_batch(
        &self,
        source: FluxproArchiveSource,
        cutoff_at: DateTime<Utc>,
        limit: i64,
    ) -> anyhow::Result<Vec<FluxproArchiveRecord>> {
        let query = match source {
            FluxproArchiveSource::ExecutionLog => {
                r#"select uuid, created_at as event_at, to_jsonb(log) as data
                   from fluxpro.process_instance_log log
                   where created_at < $1
                   order by created_at, uuid
                   limit $2"#
            }
            FluxproArchiveSource::SignalHistory => {
                // `get_last_signal` uses the newest row for duplicate protection. Keep that
                // row while its process is active; all older rows and every row of a final
                // process are technical history and can be archived.
                r#"select signal.uuid, signal.created_at as event_at,
                          to_jsonb(signal) as data
                   from fluxpro.signal_history signal
                   left join fluxpro.process_instance instance
                     on instance.token = signal.process_id
                   left join fluxpro.process_stage stage
                     on stage.uuid = instance.current_stage
                   where signal.created_at < $1
                     and (
                         instance.uuid is null
                         or coalesce(stage.is_final, false)
                         or exists (
                             select 1
                             from fluxpro.signal_history newer
                             where newer.process_id = signal.process_id
                               and (
                                   newer.created_at > signal.created_at
                                   or (newer.created_at = signal.created_at and newer.uuid > signal.uuid)
                               )
                         )
                     )
                   order by signal.created_at, signal.uuid
                   limit $2"#
            }
        };
        Ok(sqlx::query_as(query)
            .bind(cutoff_at)
            .bind(limit.clamp(1, 1_000))
            .fetch_all(&self.pool)
            .await?)
    }

    /// Deletes only the records whose exact identifiers were already persisted in a verified archive.
    pub async fn delete_archived(
        &self,
        source: FluxproArchiveSource,
        record_ids: &[Uuid],
        cutoff_at: DateTime<Utc>,
    ) -> anyhow::Result<u64> {
        if record_ids.is_empty() {
            return Ok(0);
        }
        let query = match source {
            FluxproArchiveSource::ExecutionLog => {
                "delete from fluxpro.process_instance_log where uuid = any($1) and created_at < $2"
            }
            FluxproArchiveSource::SignalHistory => {
                r#"delete from fluxpro.signal_history signal
                   where signal.uuid = any($1)
                     and signal.created_at < $2
                     and (
                         not exists (
                             select 1
                             from fluxpro.process_instance instance
                             where instance.token = signal.process_id
                         )
                         or exists (
                             select 1
                             from fluxpro.process_instance instance
                             left join fluxpro.process_stage stage
                               on stage.uuid = instance.current_stage
                             where instance.token = signal.process_id
                               and (
                                   coalesce(stage.is_final, false)
                                   or exists (
                                       select 1
                                       from fluxpro.signal_history newer
                                       where newer.process_id = signal.process_id
                                         and (
                                             newer.created_at > signal.created_at
                                             or (newer.created_at = signal.created_at and newer.uuid > signal.uuid)
                                         )
                                   )
                               )
                         )
                     )"#
            }
        };
        Ok(sqlx::query(query)
            .bind(record_ids)
            .bind(cutoff_at)
            .execute(&self.pool)
            .await?
            .rows_affected())
    }
}
