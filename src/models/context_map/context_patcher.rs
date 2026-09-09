//! Ordered operations for changing an in-memory process context.

use crate::models::context_map::context_map::ContextValue;
use crate::models::context_map::context_patch_builder::ContextPatchBuilder;
use crate::models::id_field::IdField;

/// Ordered changes applied to an in-memory `ContextMap`.
///
/// Queued handler patches persist ordered sets and removals atomically.
/// The low-level context map writer remains a merge operation.
///
/// # Examples
///
/// ```
/// use fluxpro_engine::models::context_map::context_map::ContextMap;
/// use fluxpro_engine::models::context_map::context_patcher::ContextPatcher;
/// use fluxpro_engine::models::id_field::IdField;
///
/// let approved = IdField::new("approved").unwrap();
/// let patch = ContextPatcher::builder()
///     .set_bool(approved.clone(), false)
///     .set_bool(approved.clone(), true)
///     .build();
/// let mut context = ContextMap::default();
/// context.apply_patcher(&patch);
/// assert_eq!(context.as_bool(&approved), Some(true));
/// ```
#[derive(Default)]
pub struct ContextPatcher {
    ops: Vec<ContextPatcherOp>,
}

impl ContextPatcher {
    /// Creates a patch preserving the supplied operation order.
    pub fn new(ops: Vec<ContextPatcherOp>) -> Self {
        Self { ops }
    }

    /// Creates an empty patch builder.
    pub fn builder() -> ContextPatchBuilder {
        ContextPatchBuilder::new()
    }

    /// Borrows the patch operations in application order.
    pub fn ops(&self) -> &Vec<ContextPatcherOp> {
        &self.ops
    }
}

/// One ordered context mutation; later operations on the same key win.
pub enum ContextPatcherOp {
    /// Inserts or replaces one context key.
    Set {
        /// Context key affected by this operation.
        key: IdField,
        /// Typed value assigned to the context key.
        value: ContextValue,
    },
    /// Removes one key from the context and its persisted default scope.
    Remove {
        /// Context key affected by this operation.
        key: IdField,
    },
}
