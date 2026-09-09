//! Stored context variable rows and typed JSON decoding.

use crate::models::context_map::context_map::ContextValue;
use crate::models::id_field::IdField;
use serde::Serialize;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, Row};
use uuid::Uuid;

/// One scoped, typed context variable read from PostgreSQL.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessInstanceContextVariableDb {
    /// Database row identity.
    pub uuid: Uuid,
    /// Database identity of the owning process instance.
    pub process_instance_uuid: Uuid,
    /// Variable namespace; runtime writes normally use `_`.
    pub scope: String,
    /// Context variable name, unique within its stored scope.
    pub name: IdField,
    /// Typed context value decoded from the JSON column.
    pub value: ContextValue,
}

impl<'r> FromRow<'r, PgRow> for ProcessInstanceContextVariableDb {
    fn from_row(row: &'r PgRow) -> sqlx::Result<Self> {
        let key: String = row.try_get("name")?;
        let value = row.try_get("value")?;
        Ok(Self {
            uuid: row.get("uuid"),
            process_instance_uuid: row.get("process_instance_uuid"),
            scope: row.get("scope"),
            name: IdField::new(key.as_str()).map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
            value: serde_json::from_value(value).map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
        })
    }
}
