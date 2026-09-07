use crate::engine::fluxpro_handlers::FluxproHandlersContainer;
use std::sync::Arc;

#[derive(Clone)]
pub struct RunnerConfig {
    pub concurrency: usize,
    pub idle_backoff_ms: u64,
    /// How long a worker owns a task without a successful heartbeat.
    pub task_lease_ms: u64,
    /// How often an active worker extends its task lease.
    pub heartbeat_interval_ms: u64,
    pub container: Arc<FluxproHandlersContainer>,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self {
            concurrency: 4,
            idle_backoff_ms: 750,
            task_lease_ms: 60_000,
            heartbeat_interval_ms: 20_000,
            container: Arc::new(FluxproHandlersContainer::default()),
        }
    }
}
impl RunnerConfig {
    pub fn with_handlers(container: Arc<FluxproHandlersContainer>) -> Self {
        Self {
            container,
            ..Default::default()
        }
    }

    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency.max(1);
        self
    }

    pub fn with_idle_backoff_ms(mut self, idle_backoff_ms: u64) -> Self {
        self.idle_backoff_ms = idle_backoff_ms.max(1);
        self
    }
}
