use crate::models::process_def::ProcessDefinition;
use crate::models::process_def_error::CreateProcessError;
use chrono::{DateTime, Utc};
use log::error;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, FromRow, PartialEq)]
pub struct ProcessDefDb {
    pub uuid: Uuid,
    pub created_at: DateTime<Utc>,
    pub key_: String,
    pub version: String,
    pub index_id: i32,
    pub status: String,
    pub effective_from: DateTime<Utc>,
    pub deprecated_at: Option<DateTime<Utc>>,
    pub version_comment: Option<String>,
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
