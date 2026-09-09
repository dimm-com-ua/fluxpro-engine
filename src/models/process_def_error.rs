//! Errors shared by definition creation, instance startup, and signal submission.

use crate::models::process_id::ProcessId;
use serde::Serialize;
use std::fmt::Display;

/// Legacy outcome wrapper for definition creation.
pub enum CreateProcessResult {
    /// Successful completion carrying its result or context patch.
    Success(ProcessId),
    /// An operation failed; associated data describes the failure.
    Error(CreateProcessError),
}

/// Definition, startup, persistence, or signal admission failure.
#[derive(Debug, Serialize)]
pub enum CreateProcessError {
    /// The definition could not be compiled.
    CompilationError,
    /// Accumulated structural definition errors.
    ValidationError(Vec<String>),
    /// Persistence failure with a diagnostic message.
    DbError(String),
    /// No usable definition or required startup declaration was found.
    ProcessDefNotFound,
    /// The process key failed validation.
    ProcessKeyNotValid,
    /// The stored version is malformed.
    ProcessVersionNotValid,
    /// The lifecycle status failed validation.
    StatusNotValid,
    /// Definition metadata failed validation.
    MetadataNotValid,
    /// The complete stored definition could not be decoded.
    FullDefinitionNotValid,
    /// The process instance could not be created.
    RunProcessError,
    /// The business process ID is already in use.
    ProcessIdNonUnique,
    /// Signal admission or queue insertion failed.
    CreateSignalError(String),
    /// The signal is neither declared nor referenced by a compatible wait node.
    SignalNotExists,
}

#[cfg(feature = "db")]
impl From<sqlx::Error> for CreateProcessError {
    fn from(value: sqlx::Error) -> Self {
        CreateProcessError::DbError(format!("db_error: {}", value))
    }
}

impl Display for CreateProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
