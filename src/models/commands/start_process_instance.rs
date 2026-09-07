use crate::models::context_map::context_map::ContextMap;
use crate::models::id_field::IdField;
use crate::models::version_id::VersionId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct StartProcessInstance {
    pub process_id: IdField,
    pub version: Option<VersionId>,
    pub context: ContextMap,
}

#[derive(Serialize)]
pub struct StartProcessInstanceResponse {
    pub process_instance_id: IdField,
}
