use crate::models::context_map::context_map::ContextMap;
use crate::models::handle_node_result::HandleNodeResult;
use crate::models::id_field::IdField;
use async_trait::async_trait;

#[async_trait]
pub trait FluxproServiceHandler {
    fn get_name(&self) -> IdField;
    async fn process_node(
        &self,
        process_token: &IdField,
        context: &ContextMap,
        args: Option<&ContextMap>,
    ) -> anyhow::Result<HandleNodeResult>;
}
