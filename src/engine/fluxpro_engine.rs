use crate::config::config::FluxProConfig;
use crate::db_service::FluxproDbServiceImpl;
use crate::engine::fluxpro_runner::config::RunnerConfig;
use crate::engine::fluxpro_runner::runner::EngineRunner;

use crate::service::process_service::{FluxproService, FluxproServiceImpl};
use sqlx::{Pool, Postgres};
use std::sync::Arc;
use tokio::sync::Notify;

pub struct FluxProEngine {
    _config: FluxProConfig,
    pub service: Arc<dyn FluxproService + Send + Sync>,
    queue_wakeup: Arc<Notify>,
}

impl FluxProEngine {
    pub fn create(db_pool: Pool<Postgres>) -> Self {
        let config = FluxProConfig::new();
        let queue_wakeup = Arc::new(Notify::new());
        let service = Arc::new(FluxproServiceImpl::with_queue_wakeup(
            Arc::new(FluxproDbServiceImpl::new(db_pool)),
            queue_wakeup.clone(),
            config.signal_retry_policy.clone(),
        ));

        Self {
            _config: config,
            service,
            queue_wakeup,
        }
    }

    pub fn make_runner(&self, cfg: RunnerConfig) -> EngineRunner {
        EngineRunner::new(self.service.clone(), self.queue_wakeup.clone(), cfg)
    }
}
