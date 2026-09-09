//! Worker concurrency, polling, lease timing, and handler configuration.

use crate::engine::fluxpro_handlers::FluxproHandlersContainer;
use std::sync::Arc;

/// Worker settings; defaults are four workers and a 750 ms polling fallback.
#[derive(Clone)]
pub struct RunnerConfig {
    /// Maximum simultaneous workers; must be greater than zero.
    pub concurrency: usize,
    /// Fallback polling interval in milliseconds; default is 750.
    pub idle_backoff_ms: u64,
    /// How long a worker owns a task without a successful heartbeat.
    pub task_lease_ms: u64,
    /// How often an active worker extends its task lease.
    pub heartbeat_interval_ms: u64,
    /// Registry of service handlers and lifecycle callbacks used by workers.
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
    /// Uses the supplied registry with the default runner timing and concurrency.
    pub fn with_handlers(container: Arc<FluxproHandlersContainer>) -> Self {
        Self {
            container,
            ..Default::default()
        }
    }

    /// Sets maximum workers, clamping zero to one.
    pub fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency.max(1);
        self
    }

    /// Sets fallback polling delay in milliseconds, clamping zero to one.
    pub fn with_idle_backoff_ms(mut self, idle_backoff_ms: u64) -> Self {
        self.idle_backoff_ms = idle_backoff_ms.max(1);
        self
    }
}
