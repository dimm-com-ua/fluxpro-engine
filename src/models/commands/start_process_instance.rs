//! Instance startup commands and the returned runtime token.

use crate::models::context_map::context_map::ContextMap;
use crate::models::id_field::IdField;
use crate::models::version_id::VersionId;
use serde::{Deserialize, Serialize};

/// Startup request identifying a business instance and its initial context.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct StartProcessInstance {
    /// Application business identifier; distinct from the generated runtime token.
    pub process_id: IdField,
    /// Requested version; omitted selects the latest inserted eligible definition.
    pub version: Option<VersionId>,
    /// Initial variables stored in the instance context before Start is queued.
    pub context: ContextMap,
}

/// Startup response containing the generated runtime token.
#[derive(Serialize)]
pub struct StartProcessInstanceResponse {
    /// Generated runtime token used to address this instance.
    pub process_instance_id: IdField,
}
