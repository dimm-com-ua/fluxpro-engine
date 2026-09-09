//! Example host handler shared by the standalone handler and workflow demos.

use async_trait::async_trait;
use fluxpro_engine::models::context_map::context_map::ContextMap;
use fluxpro_engine::models::context_map::context_patcher::ContextPatcher;
use fluxpro_engine::models::handle_node_result::HandleNodeResult;
use fluxpro_engine::models::id_field::IdField;
use fluxpro_engine::traits::node_handlers::service_node_handler::FluxproServiceHandler;

/// Marks a request as prepared using a repeatable context assignment.
pub struct PrepareRequest;

#[async_trait]
impl FluxproServiceHandler for PrepareRequest {
    fn get_name(&self) -> IdField {
        IdField::new("prepare_request").expect("the static handler ID is valid")
    }

    async fn process_node(
        &self,
        _process_token: &IdField,
        _context: &ContextMap,
        _args: Option<&ContextMap>,
    ) -> anyhow::Result<HandleNodeResult> {
        // Assigning a stable value remains safe if the queue repeats this handler.
        let patch = ContextPatcher::builder()
            .set_bool(IdField::new("prepared")?, true)
            .build();
        Ok(HandleNodeResult::success_with_patcher(patch))
    }
}
