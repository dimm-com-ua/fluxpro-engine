//! Queue payload serialization and task execution outcomes.

use crate::models::commands::post_signal::PostSignal;
use crate::models::id_field::IdField;
use crate::models::process_def::Node;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "db")]
use log::error;
#[cfg(feature = "db")]
use sqlx::postgres::PgRow;
#[cfg(feature = "db")]
use sqlx::{Error, FromRow, Row};

/// Persistent work item for node entry, timeout routing, or signal delivery.
///
/// Serde tags are part of the SQL queue contract: `process_node`,
/// `ProcessEvent`, and `ProcessSignal`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FluxproQueueTask {
    /// Enters and executes a node.
    #[serde(rename = "process_node")]
    ProcessNode {
        /// Generated runtime token identifying the owning process instance.
        process_token: IdField,
        /// Snapshot of the node associated with this queue item.
        node: Node,
    },
    /// Routes a timeout if its originating node is still current.
    ProcessEvent {
        /// Generated runtime token identifying the owning process instance.
        process_token: IdField,
        /// Snapshot of the node associated with this queue item.
        node: Node,
        /// Timeout successor node ID; despite the name, this is not a signal.
        on_time: IdField,
    },
    /// Delivers a signal when the current node accepts its name.
    ProcessSignal {
        /// Generated runtime token identifying the owning process instance.
        process_token: IdField,
        /// Signal name accepted by a waiting node.
        signal: PostSignal,
    },
}

impl FluxproQueueTask {
    /// Borrows the runtime instance token associated with this task.
    pub fn process_token(&self) -> &IdField {
        match self {
            Self::ProcessNode { process_token, .. }
            | Self::ProcessEvent { process_token, .. }
            | Self::ProcessSignal { process_token, .. } => process_token,
        }
    }
}

/// Persisted task payload with scheduling and lease ownership metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FluxproQueueTaskDefinition {
    /// Database row identity.
    pub uuid: Uuid,
    /// UTC time when the row was created.
    pub created_at: DateTime<Utc>,
    /// Earliest UTC time at which the task may be claimed.
    pub run_after: DateTime<Utc>,
    /// Number of queue claims, including the current attempt.
    pub attempts: i32,
    /// Lease ownership key; an empty string indicates an unleased task.
    pub lock_key: String,
    /// UTC time at which the current lease was acquired.
    pub locked_at: Option<DateTime<Utc>>,
    /// Legacy field name read from `locked_by`; decode mismatches yield `None`.
    ///
    /// The database column stores a lease deadline, not a worker identity.
    /// Use the administration model for a typed lease-expiry timestamp.
    pub locket_by: Option<String>,
    /// Serialized or decoded queue work payload.
    pub task: FluxproQueueTask,
}

/// Whether a queue item was consumed or retained for a later attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskProcessOutcome {
    /// The queue item was consumed and removed.
    Completed,
    /// The queue item remains scheduled for a later attempt.
    RetryScheduled,
    /// The instance is suspended with this task retained for explicit recovery.
    Suspended,
}

impl FluxproQueueTaskDefinition {
    /// Returns the stable task-kind label used in diagnostics.
    pub fn kind(&self) -> String {
        match self.task {
            FluxproQueueTask::ProcessNode { .. } => "process_node".to_string(),
            FluxproQueueTask::ProcessEvent { .. } => "process_event".to_string(),
            FluxproQueueTask::ProcessSignal { .. } => "process_signal".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::context_map::context_map::ContextMap;

    #[test]
    fn queue_task_process_token_paths_match_db_serialization_contract() {
        let process_token = IdField::new("process_1").unwrap();
        let node = Node::Start {
            id: IdField::new("start").unwrap(),
            next: IdField::new("next").unwrap(),
            set_stage: None,
        };
        let signal = PostSignal {
            event_id: None,
            wait_visit_id: None,
            signal: IdField::new("continue").unwrap(),
            context: ContextMap::default(),
        };

        let process_node = serde_json::to_value(FluxproQueueTask::ProcessNode {
            process_token: process_token.clone(),
            node: node.clone(),
        })
        .unwrap();
        let process_event = serde_json::to_value(FluxproQueueTask::ProcessEvent {
            process_token: process_token.clone(),
            node,
            on_time: IdField::new("timeout").unwrap(),
        })
        .unwrap();
        let process_signal = serde_json::to_value(FluxproQueueTask::ProcessSignal {
            process_token,
            signal,
        })
        .unwrap();

        assert_eq!(
            process_node.pointer("/process_node/process_token"),
            Some(&serde_json::json!("process_1"))
        );
        assert_eq!(
            process_event.pointer("/ProcessEvent/process_token"),
            Some(&serde_json::json!("process_1"))
        );
        assert_eq!(
            process_signal.pointer("/ProcessSignal/process_token"),
            Some(&serde_json::json!("process_1"))
        );
    }
}

#[cfg(feature = "db")]
impl<'r> FromRow<'r, PgRow> for FluxproQueueTaskDefinition {
    fn from_row(row: &'r PgRow) -> Result<Self, Error> {
        let uuid = row.try_get("uuid")?;
        let created_at = row.try_get("created_at")?;
        let run_after = row.try_get("run_after")?;
        let attempts = row.try_get("attempts")?;
        let lock_key = row.try_get("lock_key")?;
        let locked_at = row.try_get("locked_at").unwrap_or(None);
        let locket_by = row.try_get("locked_by").unwrap_or(None);
        let task_def = row.try_get("task")?;
        let task = serde_json::from_value::<FluxproQueueTask>(task_def).map_err(|e| {
            error!("Error decoding task definition: {}", e);
            sqlx::Error::Decode(Box::new(e))
        })?;

        Ok(FluxproQueueTaskDefinition {
            uuid,
            created_at,
            run_after,
            attempts,
            lock_key,
            locked_at,
            locket_by,
            task,
        })
    }
}
