//! Signal submission with a typed context update.

use crate::models::context_map::context_map::ContextMap;
use crate::models::id_field::IdField;
use serde::{Deserialize, Serialize};

/// A signal name and typed context values to merge when the signal is accepted.
#[derive(Deserialize, Debug, Serialize, Clone, PartialEq)]
pub struct PostSignal {
    /// Producer event identity, unique within the instance; omitted events are independent deliveries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    /// Expected wait visit; use this to reject a delayed event after reentering a node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_visit_id: Option<uuid::Uuid>,
    /// Signal name accepted by a waiting node.
    pub signal: IdField,
    /// Variables merged into context only when a waiting node accepts this signal.
    pub context: ContextMap,
}
