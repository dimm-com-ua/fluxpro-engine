//! Validates or runs business scenarios with simulated integrations; requires `api`.
#[path = "support/business.rs"]
mod business;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str).unwrap_or("list") {
        "list" => {
            for scenario in business::scenarios()? {
                println!("{} ({})", scenario.name, scenario.workflow);
            }
        }
        "validate" => {
            for (name, _) in business::DEFINITIONS {
                let definition = business::definition(name)?;
                definition.validate().map_err(|e| anyhow::anyhow!("{e}"))?;
                anyhow::ensure!(
                    definition.unreachable_nodes().is_empty(),
                    "{name} has unreachable nodes"
                );
                println!("Validated {name}: {} nodes", definition.nodes.len());
            }
        }
        "run" => {
            let name = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("usage: business_workflows run SCENARIO"))?;
            let scenario = business::scenarios()?
                .into_iter()
                .find(|s| &s.name == name)
                .ok_or_else(|| anyhow::anyhow!("unknown scenario {name}; use list"))?;
            let pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(5)
                .connect(&std::env::var("DATABASE_URL")?)
                .await?;
            let token = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                business::run(pool, scenario),
            )
            .await??;
            println!(
                "Verified {name}; instance {token}. Host results were simulated; no external services called."
            );
        }
        _ => anyhow::bail!("usage: business_workflows [list|validate|run SCENARIO]"),
    }
    Ok(())
}
