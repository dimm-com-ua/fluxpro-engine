//! Validates and compiles a JSON definition without integration features or a database.

use fluxpro_engine::models::process_def::ProcessDefinition;

fn main() -> anyhow::Result<()> {
    let definition: ProcessDefinition =
        serde_json::from_str(include_str!("definitions/minimal.json"))?;
    let compiled = definition
        .compile()
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    println!("{}", serde_json::to_string_pretty(&compiled)?);
    Ok(())
}
