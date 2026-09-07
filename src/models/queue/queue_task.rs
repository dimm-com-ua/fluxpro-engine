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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FluxproQueueTask {
    #[serde(rename = "process_node")]
    ProcessNode { process_token: IdField, node: Node },
    ProcessEvent {
        process_token: IdField,
        node: Node,
        on_time: IdField,
    },
    ProcessSignal {
        process_token: IdField,
        signal: PostSignal,
    },
}

impl FluxproQueueTask {
    pub fn process_token(&self) -> &IdField {
        match self {
            Self::ProcessNode { process_token, .. }
            | Self::ProcessEvent { process_token, .. }
            | Self::ProcessSignal { process_token, .. } => process_token,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FluxproQueueTaskDefinition {
    pub uuid: Uuid,
    pub created_at: DateTime<Utc>,
    pub run_after: DateTime<Utc>,
    pub attempts: i32,
    pub lock_key: String,
    pub locked_at: Option<DateTime<Utc>>,
    pub locket_by: Option<String>,
    pub task: FluxproQueueTask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskProcessOutcome {
    Completed,
    RetryScheduled,
}

impl FluxproQueueTaskDefinition {
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
