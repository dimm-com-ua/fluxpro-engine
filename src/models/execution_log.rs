use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt::{Display, Formatter};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionLogLevel {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

impl Display for ExecutionLogLevel {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Critical => "critical",
        };
        formatter.write_str(value)
    }
}

/// A durable event in the execution history of one process instance.
///
/// `attempt` and the error fields are intentionally independent from retry
/// policy. They preserve enough history for retry limits and terminal instance
/// errors to be implemented without changing the log schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionLogEvent {
    pub level: ExecutionLogLevel,
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

impl ExecutionLogEvent {
    pub fn info(
        event_type: impl Into<String>,
        source: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            level: ExecutionLogLevel::Info,
            event_type: event_type.into(),
            source: source.into(),
            message: message.into(),
            node_id: None,
            handler_id: None,
            queue_task_uuid: None,
            attempt: None,
            error_kind: None,
            error_message: None,
            details: json!({}),
        }
    }

    pub fn warning(
        event_type: impl Into<String>,
        source: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let mut event = Self::info(event_type, source, message);
        event.level = ExecutionLogLevel::Warning;
        event
    }

    pub fn critical(
        event_type: impl Into<String>,
        source: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let mut event = Self::info(event_type, source, message);
        event.level = ExecutionLogLevel::Critical;
        event
    }

    pub fn error(
        event_type: impl Into<String>,
        source: impl Into<String>,
        error: &anyhow::Error,
    ) -> Self {
        Self {
            level: ExecutionLogLevel::Error,
            event_type: event_type.into(),
            source: source.into(),
            message: "Process execution failed".into(),
            node_id: None,
            handler_id: None,
            queue_task_uuid: None,
            attempt: None,
            error_kind: Some(std::any::type_name_of_val(error).into()),
            error_message: Some(format!("{error:#}")),
            details: json!({ "error_chain": error.chain().map(ToString::to_string).collect::<Vec<_>>() }),
        }
    }
}
