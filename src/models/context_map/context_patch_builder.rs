use crate::models::context_map::context_map::ContextValue;
use crate::models::context_map::context_patcher::{ContextPatcher, ContextPatcherOp};
use crate::models::id_field::IdField;

pub struct ContextPatchBuilder {
    ops: Vec<ContextPatcherOp>,
}

impl ContextPatchBuilder {
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
    pub fn set(mut self, key: IdField, value: ContextValue) -> Self {
        self.ops.push(ContextPatcherOp::Set { key, value });
        self
    }

    pub fn remove(mut self, key: IdField) -> Self {
        self.ops.push(ContextPatcherOp::Remove { key });
        self
    }

    pub fn set_string(mut self, key: IdField, value: String) -> Self {
        self.ops.push(ContextPatcherOp::Set {
            key,
            value: ContextValue::String { string: value },
        });
        self
    }

    pub fn set_number(mut self, key: IdField, value: i64) -> Self {
        self.ops.push(ContextPatcherOp::Set {
            key,
            value: ContextValue::Number { number: value },
        });
        self
    }

    pub fn set_float(mut self, key: IdField, value: f64) -> Self {
        self.ops.push(ContextPatcherOp::Set {
            key,
            value: ContextValue::Float { float: value },
        });
        self
    }

    pub fn set_bool(mut self, key: IdField, value: bool) -> Self {
        self.ops.push(ContextPatcherOp::Set {
            key,
            value: ContextValue::Boolean { boolean: value },
        });
        self
    }

    pub fn build(self) -> ContextPatcher {
        ContextPatcher::new(self.ops)
    }
}
