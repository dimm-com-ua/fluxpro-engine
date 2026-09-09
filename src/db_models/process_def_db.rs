//! Stored definition rows and conversion back to workflow model types.

use crate::models::process_def::ProcessDefinition;
use crate::models::process_def_error::CreateProcessError;
use chrono::{DateTime, Utc};
use log::error;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

/// Database row containing a compiled definition and version metadata.
#[derive(Debug, FromRow, PartialEq)]
pub struct ProcessDefDb {
    /// Database row identity.
    pub uuid: Uuid,
    /// UTC time when the row was created.
    pub created_at: DateTime<Utc>,
    /// Process definition key stored under the SQL column name.
    pub key_: String,
    /// Normalized process definition version.
    pub version: String,
    /// Insertion-order index for versions of the same process key.
    pub index_id: i32,
    /// Stored lifecycle status of this definition version.
    pub status: String,
    /// UTC time from which this definition can be selected.
    pub effective_from: DateTime<Utc>,
    /// UTC time after which this definition is excluded from runtime selection.
    pub deprecated_at: Option<DateTime<Utc>>,
    /// Human-readable notes supplied by the version metadata.
    pub version_comment: Option<String>,
    /// Compiled workflow definition as JSON.
    pub definition: Value,
}

impl TryInto<ProcessDefinition> for ProcessDefDb {
    type Error = CreateProcessError;

    fn try_into(self) -> Result<ProcessDefinition, Self::Error> {
        let full_def: ProcessDefinition = serde_json::from_value(self.definition).map_err(|e| {
            error!(
                "Error while converting definition to process definition: {}",
                e
            );
            CreateProcessError::FullDefinitionNotValid
        })?;
        let with_uuid = ProcessDefinition {
            uuid: Some(self.uuid),
            ..full_def
        };
        Ok(with_uuid)
    }
}
