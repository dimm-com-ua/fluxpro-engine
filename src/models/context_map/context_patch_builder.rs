//! Fluent construction of ordered context updates.

use crate::models::context_map::context_map::ContextValue;
use crate::models::context_map::context_patcher::{ContextPatcher, ContextPatcherOp};
use crate::models::id_field::IdField;

/// Fluent builder for ordered context set and remove operations.
pub struct ContextPatchBuilder {
    ops: Vec<ContextPatcherOp>,
}

impl ContextPatchBuilder {
    /// Creates an empty ordered patch builder.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }
    /// Adds a typed value assignment; a later assignment to the same key wins.
    pub fn set(mut self, key: IdField, value: ContextValue) -> Self {
        self.ops.push(ContextPatcherOp::Set { key, value });
        self
    }

    /// Adds an operation that removes a key from the in-memory context.
    pub fn remove(mut self, key: IdField) -> Self {
        self.ops.push(ContextPatcherOp::Remove { key });
        self
    }

    /// Appends a string assignment to the patch.
    pub fn set_string(mut self, key: IdField, value: String) -> Self {
        self.ops.push(ContextPatcherOp::Set {
            key,
            value: ContextValue::String { string: value },
        });
        self
    }

    /// Appends a number assignment to the patch.
    pub fn set_number(mut self, key: IdField, value: i64) -> Self {
        self.ops.push(ContextPatcherOp::Set {
            key,
            value: ContextValue::Number { number: value },
        });
        self
    }

    /// Appends a float assignment to the patch.
    pub fn set_float(mut self, key: IdField, value: f64) -> Self {
        self.ops.push(ContextPatcherOp::Set {
            key,
            value: ContextValue::Float { float: value },
        });
        self
    }

    /// Appends a bool assignment to the patch.
    pub fn set_bool(mut self, key: IdField, value: bool) -> Self {
        self.ops.push(ContextPatcherOp::Set {
            key,
            value: ContextValue::Boolean { boolean: value },
        });
        self
    }

    /// Consumes the builder and returns its accumulated value.
    pub fn build(self) -> ContextPatcher {
        ContextPatcher::new(self.ops)
    }
}
