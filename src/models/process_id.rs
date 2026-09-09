//! Identifier returned after a process definition is persisted.

use serde::Serialize;

/// Response wrapper for the UUID string of a persisted process definition.
#[derive(Serialize)]
pub struct ProcessId {
    /// UUID string of the persisted process definition.
    pub id: String,
}

impl ProcessId {
    /// Wraps the persisted definition ID without additional validation.
    pub fn new(id: String) -> Self {
        Self { id }
    }
}
