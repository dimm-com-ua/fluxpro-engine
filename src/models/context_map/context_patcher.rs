use crate::models::context_map::context_map::ContextValue;
use crate::models::context_map::context_patch_builder::ContextPatchBuilder;
use crate::models::id_field::IdField;

#[derive(Default)]
pub struct ContextPatcher {
    ops: Vec<ContextPatcherOp>,
}

impl ContextPatcher {
    pub fn new(ops: Vec<ContextPatcherOp>) -> Self {
        Self { ops }
    }

    pub fn builder() -> ContextPatchBuilder {
        ContextPatchBuilder::new()
    }

    pub fn ops(&self) -> &Vec<ContextPatcherOp> {
        &self.ops
    }
}

pub enum ContextPatcherOp {
    Set { key: IdField, value: ContextValue },
    Remove { key: IdField },
}
