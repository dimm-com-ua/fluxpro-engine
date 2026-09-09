//! Assembly of the database service and runner notification channel.

use crate::config::config::FluxProConfig;
use crate::db_service::FluxproDbServiceImpl;
use crate::engine::fluxpro_runner::config::RunnerConfig;
use crate::engine::fluxpro_runner::runner::EngineRunner;

use crate::service::process_service::{FluxproService, FluxproServiceImpl};
use sqlx::{Pool, Postgres};
use std::sync::Arc;
use tokio::sync::Notify;

/// Runtime facade sharing one workflow service and local runner notifications.
pub struct FluxProEngine {
    _config: FluxProConfig,
    /// Shared workflow operations used by the host and queue runner.
    pub service: Arc<dyn FluxproService + Send + Sync>,
    queue_wakeup: Arc<Notify>,
}

impl FluxProEngine {
    /// Builds the engine with default policy and a supplied PostgreSQL pool.
    ///
    /// Apply migrations separately before performing workflow operations.
    pub fn create(db_pool: Pool<Postgres>) -> Self {
        Self::create_with_handler_timeout(db_pool, std::time::Duration::from_secs(300))
            .expect("default handler timeout is positive")
    }

    /// Builds the engine with a deadline for each cooperative asynchronous host call.
    ///
    /// Timeout suspends the instance because an external operation may have completed.
    pub fn create_with_handler_timeout(
        db_pool: Pool<Postgres>,
        timeout: std::time::Duration,
    ) -> anyhow::Result<Self> {
        let config = FluxProConfig::new();
        let queue_wakeup = Arc::new(Notify::new());
        let service = Arc::new(
            FluxproServiceImpl::with_queue_wakeup(
                Arc::new(FluxproDbServiceImpl::new(db_pool)),
                queue_wakeup.clone(),
                config.signal_retry_policy.clone(),
            )
            .with_handler_timeout(timeout)?,
        );

        Ok(Self {
            _config: config,
            service,
            queue_wakeup,
        })
    }

    /// Builds a runner sharing this engine's service and wakeup channel.
    pub fn make_runner(&self, cfg: RunnerConfig) -> EngineRunner {
        EngineRunner::new(self.service.clone(), self.queue_wakeup.clone(), cfg)
    }
}
