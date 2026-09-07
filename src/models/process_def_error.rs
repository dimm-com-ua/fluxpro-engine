use crate::models::process_id::ProcessId;
use serde::Serialize;
use std::fmt::Display;

pub enum CreateProcessResult {
    Success(ProcessId),
    Error(CreateProcessError),
}

#[derive(Debug, Serialize)]
pub enum CreateProcessError {
    CompilationError,
    ValidationError(Vec<String>),
    DbError(String),
    ProcessDefNotFound,
    ProcessKeyNotValid,
    ProcessVersionNotValid,
    StatusNotValid,
    MetadataNotValid,
    FullDefinitionNotValid,
    RunProcessError,
    ProcessIdNonUnique,
    CreateSignalError(String),
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
