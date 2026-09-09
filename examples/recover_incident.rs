//! Inspects a suspended instance; resumes only when the reviewed incident UUID is supplied.
use fluxpro_engine::engine::fluxpro_engine::FluxProEngine;
use fluxpro_engine::models::id_field::IdField;
use sqlx::postgres::PgPoolOptions;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let token = IdField::new(args.next().ok_or_else(|| {
        anyhow::anyhow!("usage: recover_incident INSTANCE_TOKEN [REVIEWED_INCIDENT_UUID]")
    })?)?;
    let reviewed = args
        .next()
        .map(|value| value.parse::<uuid::Uuid>())
        .transpose()?;
    anyhow::ensure!(args.next().is_none(), "unexpected extra argument");
    // Apply migrations during deployment, not implicitly from an operator command.
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&std::env::var("DATABASE_URL")?)
        .await?;
    let engine = FluxProEngine::create(pool);
    if let Some(incident_id) = reviewed {
        let resumed = engine.service.resume_instance(&token, incident_id).await?;
        println!("Resumed: {resumed}. Existing runners pick up the retained task through polling.");
    } else if let Some(incident) = engine.service.get_open_incident(&token).await? {
        println!("{}", serde_json::to_string_pretty(&incident)?);
    } else {
        println!("No unresolved incident for {token}");
    }
    Ok(())
}
