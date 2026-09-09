//! Host-provided asynchronous handlers invoked by service tasks and lifecycle hooks.

use crate::models::context_map::context_map::ContextMap;
use crate::models::handle_node_result::HandleNodeResult;
use crate::models::id_field::IdField;
use async_trait::async_trait;

/// Stable identity and ordering metadata for one host operation.
///
/// Persist `operation_id` at the side-effect boundary to deduplicate retries.
/// Reject an older `instance_revision` when updating externally stored stages.
/// Supplying metadata alone does not make a remote operation idempotent.
#[derive(Debug, Clone)]
pub struct HandlerExecution {
    /// Stable per-task/per-phase idempotency key, unchanged after replay or resume.
    pub operation_id: String,
    /// Source queue identity, also the node visit identity for node entry tasks.
    pub task_id: uuid::Uuid,
    /// Definition-local node identifier being executed.
    pub node_id: IdField,
    /// Operation phase: service, hide_form, stage, show_form, or complete.
    pub phase: &'static str,
    /// Persisted instance revision read before this attempt.
    pub instance_revision: i64,
    /// Queue claim count; an explicit resume resets the retry budget.
    pub attempt: i32,
}

/// Asynchronous host logic shared by service tasks and lifecycle callbacks.
#[async_trait]
pub trait FluxproServiceHandler {
    /// Executes with stable operation identity; legacy handlers delegate to `process_node`.
    ///
    /// Override this method to pass idempotency keys and revision fencing to the
    /// host database or external API. Return the original result for duplicate keys.
    async fn process_node_with_execution(
        &self,
        process_token: &IdField,
        context: &ContextMap,
        args: Option<&ContextMap>,
        _execution: &HandlerExecution,
    ) -> anyhow::Result<HandleNodeResult> {
        self.process_node(process_token, context, args).await
    }

    /// Returns the registry ID referenced by workflow handler declarations.
    fn get_name(&self) -> IdField;
    /// Executes host logic using the instance token, current context, and node arguments.
    ///
    /// Return a context patch on success or an error to invoke service-task retry
    /// handling. External side effects should tolerate repeated execution after
    /// failures or lease expiry. Queued lifecycle hooks require `Success`.
    async fn process_node(
        &self,
        process_token: &IdField,
        context: &ContextMap,
        args: Option<&ContextMap>,
    ) -> anyhow::Result<HandleNodeResult>;
}
