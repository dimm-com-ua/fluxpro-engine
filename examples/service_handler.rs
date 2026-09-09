//! Executes a host handler locally without PostgreSQL; requires `runtime` for Tokio.

mod support;

use fluxpro_engine::models::context_map::context_map::ContextMap;
use fluxpro_engine::models::handle_node_result::HandleResultStatus;
use fluxpro_engine::models::id_field::IdField;
use fluxpro_engine::traits::node_handlers::service_node_handler::FluxproServiceHandler;
use support::PrepareRequest;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let mut context = ContextMap::default();
    let outcome = PrepareRequest
        .process_node(&IdField::new("demo_instance")?, &context, None)
        .await?;
    let HandleResultStatus::Success(patch) = outcome.status else {
        anyhow::bail!("the example handler must succeed");
    };
    context.apply_patcher(&patch);
    assert_eq!(context.as_bool(&IdField::new("prepared")?), Some(true));
    println!("{}", serde_json::to_string_pretty(&context)?);
    Ok(())
}
