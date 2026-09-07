use crate::models::signal_delivery::SignalRetryPolicy;

#[derive(Clone, Default)]
pub struct FluxProConfig {
    pub signal_retry_policy: SignalRetryPolicy,
}

impl FluxProConfig {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Clone)]
pub struct FluxProDbConfig {}

impl FluxProDbConfig {
    pub fn new() -> Self {
        Self {}
    }
}
