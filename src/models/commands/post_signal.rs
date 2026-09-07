use crate::models::context_map::context_map::ContextMap;
use crate::models::id_field::IdField;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug, Serialize, Clone, PartialEq)]
pub struct PostSignal {
    pub signal: IdField,
    pub context: ContextMap,
}
