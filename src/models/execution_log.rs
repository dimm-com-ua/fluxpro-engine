//! Structured execution events for diagnostics and process history.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt::{Display, Formatter};
use uuid::Uuid;

/// Severity serialized as a lowercase name for execution history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionLogLevel {
    /// Detailed diagnostic information.
    Debug,
    /// Normal execution information.
    Info,
    /// A recoverable or unusual condition.
    Warning,
    /// An operation failed; associated data describes the failure.
    Error,
    /// A serious failure or detected stalled process.
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
    /// Execution event severity.
    pub level: ExecutionLogLevel,
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

impl ExecutionLogEvent {
    /// Creates an informational execution event with empty diagnostic details.
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

    /// Creates a warning execution event with empty diagnostic details.
    pub fn warning(
        event_type: impl Into<String>,
        source: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let mut event = Self::info(event_type, source, message);
        event.level = ExecutionLogLevel::Warning;
        event
    }

    /// Creates a critical execution event with empty diagnostic details.
    pub fn critical(
        event_type: impl Into<String>,
        source: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let mut event = Self::info(event_type, source, message);
        event.level = ExecutionLogLevel::Critical;
        event
    }

    /// Creates an error event retaining the error message and cause-chain details.
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
