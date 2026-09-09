//! Durable execution incidents and explicit, compare-and-set recovery.

use super::FluxproDbServiceImpl;
use crate::models::id_field::IdField;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

/// A stopped execution attempt retained until an operator explicitly resumes it.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ProcessIncident {
    /// Identity required when resuming, preventing stale or duplicate recovery commands.
    pub uuid: Uuid,
    /// Owning instance database identity.
    pub process_instance_uuid: Uuid,
    /// Queue item retained for replay.
    pub task_uuid: Uuid,
    /// Original task payload for diagnostics.
    pub task: serde_json::Value,
    /// Stable error category.
    pub kind: String,
    /// Failure diagnostic saved with the suspension.
    pub reason: String,
    /// Attempt at which execution stopped.
    pub attempt: i32,
    /// Time at which the incident was committed.
    pub created_at: DateTime<Utc>,
    /// Explicit resumption time, absent while unresolved.
    pub resolved_at: Option<DateTime<Utc>>,
}

impl FluxproDbServiceImpl {
    /// Reads the unresolved incident without deriving it from logs or node age.
    pub async fn get_open_incident(
        &self,
        token: &IdField,
    ) -> anyhow::Result<Option<ProcessIncident>> {
        Ok(sqlx::query_as("select incident.* from fluxpro.process_incident incident join fluxpro.process_instance i on i.uuid=incident.process_instance_uuid where i.token=$1 and incident.resolved_at is null")
            .bind(token.get_id()).fetch_optional(&self.db_pool).await?)
    }

    /// Resumes exactly the specified incident and resets the retained task's retry budget.
    ///
    /// Returns false if it was already resolved or superseded. Fix the cause first;
    /// replay can repeat external handler effects. Other queued work stays blocked
    /// until the failed task has successfully committed or is suspended again.
    pub async fn resume_instance(
        &self,
        token: &IdField,
        incident_id: Uuid,
    ) -> anyhow::Result<bool> {
        let mut tx = self.db_pool.begin().await?;
        let instance: Uuid = sqlx::query_scalar(
            "select uuid from fluxpro.process_instance where token=$1 for update",
        )
        .bind(token.get_id())
        .fetch_one(&mut *tx)
        .await?;
        let incident = sqlx::query("select task_uuid from fluxpro.process_incident where uuid=$1 and process_instance_uuid=$2 and resolved_at is null for update")
            .bind(incident_id).bind(instance).fetch_optional(&mut *tx).await?;
        let Some(incident) = incident else {
            // Drop only queues SQLx's rollback. Release the instance row lock
            // before returning, so an immediate SKIP LOCKED claim can see the
            // task already made runnable by an earlier successful resume.
            tx.rollback().await?;
            return Ok(false);
        };
        let task: Uuid = incident.try_get("task_uuid")?;
        let affected = sqlx::query("update fluxpro.queue_runner set attempts=0, lock_key='',locked_at=null,locked_by=null,run_after=clock_timestamp() where uuid=$1 and (locked_by is null or locked_by<clock_timestamp())")
            .bind(task).execute(&mut *tx).await?.rows_affected();
        anyhow::ensure!(affected == 1, "incident task is missing or still leased");
        sqlx::query(
            "update fluxpro.process_incident set resolved_at=clock_timestamp() where uuid=$1",
        )
        .bind(incident_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("update fluxpro.process_instance set execution_state='running',revision=revision+1,recovery_task_uuid=$2 where uuid=$1")
            .bind(instance).bind(task).execute(&mut *tx).await?;
        sqlx::query("insert into fluxpro.process_instance_log(process_instance_uuid,level,event_type,source,message,queue_task_uuid,details) values ($1,'info','instance.resumed','operator','Explicitly resumed incident task',$2,jsonb_build_object('incident_id',$3::text))")
            .bind(instance).bind(task).bind(incident_id.to_string()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }
}
