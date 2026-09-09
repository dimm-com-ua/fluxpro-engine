//! Validates the example containing every supported node type; requires `api`.

use fluxpro_engine::models::process_def::ProcessDefinition;

fn main() -> anyhow::Result<()> {
    let definition: ProcessDefinition =
        serde_yaml::from_str(include_str!("definitions/approval.yaml"))?;
    definition
        .validate()
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    println!(
        "Validated {} with {} nodes",
        definition.key,
        definition.nodes.len()
    );
    Ok(())
}
