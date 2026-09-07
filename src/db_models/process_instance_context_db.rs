use crate::models::context_map::context_map::ContextValue;
use crate::models::id_field::IdField;
use serde::Serialize;
use sqlx::postgres::PgRow;
use sqlx::{FromRow, Row};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct ProcessInstanceContextVariableDb {
    pub uuid: Uuid,
    pub process_instance_uuid: Uuid,
    pub scope: String,
    pub name: IdField,
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
