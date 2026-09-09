//! Runs the approval workflow against PostgreSQL; requires `api` and `DATABASE_URL`.

mod support;

use fluxpro_engine::engine::fluxpro_engine::FluxProEngine;
use fluxpro_engine::engine::fluxpro_handlers::FluxproHandlersContainer;
use fluxpro_engine::engine::fluxpro_runner::config::RunnerConfig;
use fluxpro_engine::engine::fluxpro_runner::shutdown::Shutdown;
use fluxpro_engine::models::commands::post_signal::PostSignal;
use fluxpro_engine::models::commands::start_process_instance::StartProcessInstance;
use fluxpro_engine::models::context_map::context_map::ContextMap;
use fluxpro_engine::models::id_field::IdField;
use fluxpro_engine::models::process_def::ProcessDefinition;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use std::time::Duration;
use support::PrepareRequest;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let database_url = std::env::var("DATABASE_URL")?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await?;
    fluxpro_engine::migrations::migrate(&pool).await?;
    let engine = FluxProEngine::create(pool);
    let source = include_str!("definitions/approval.yaml");
    let definition: ProcessDefinition = serde_yaml::from_str(source)?;
    engine
        .service
        .create_process_def_from_source(&definition, source)
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let token = engine
        .service
        .start_process_instance(
            definition.key.clone(),
            StartProcessInstance {
                process_id: IdField::generate(),
                version: Some(definition.version.clone()),
                context: ContextMap::default(),
            },
        )
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    let handlers = Arc::new(FluxproHandlersContainer::new(vec![Arc::new(
        PrepareRequest,
    )]));
    let runner = engine.make_runner(RunnerConfig::with_handlers(handlers));
    let shutdown = Shutdown::new();
    let runner_shutdown = shutdown.clone();
    let worker = tokio::spawn(async move { runner.run_until_stopped(runner_shutdown).await });

    // Wait for each accepting node so this demo also shows the intended signal sequence.
    let execution = tokio::time::timeout(Duration::from_secs(30), async {
        for (node, signal) in [("review", "approved"), ("await_confirmation", "confirmed")] {
            wait_for_node(&engine, &token, node).await?;
            engine
                .service
                .post_signal(
                    token.clone(),
                    PostSignal {
                        event_id: Some(format!("{node}-{signal}")),
                        wait_visit_id: None,
                        signal: IdField::new(signal)?,
                        context: ContextMap::default(),
                    },
                )
                .await
                .map_err(|error| anyhow::anyhow!("{error}"))?;
        }
        wait_for_node(&engine, &token, "completed_end").await?;
        Ok::<_, anyhow::Error>(())
    })
    .await;
    shutdown.trigger();
    worker.await??;
    execution??;
    println!("Completed instance {token}");
    Ok(())
}

/// Polls the persisted current node; startup may briefly have no current node.
async fn wait_for_node(
    engine: &FluxProEngine,
    token: &IdField,
    expected: &str,
) -> anyhow::Result<()> {
    loop {
        if let Ok(Some(node)) = engine
            .service
            .get_process_instance_current_node(token)
            .await
        {
            if node.id().get_id() == expected {
                return Ok(());
            }
            anyhow::ensure!(
                !node.is_end(),
                "process ended at {} before {expected}",
                node.id()
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
