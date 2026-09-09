//! Engine defaults, including durable signal retry policy.

use crate::models::signal_delivery::SignalRetryPolicy;

/// Default engine configuration used when assembling the runtime.
#[derive(Clone, Default)]
pub struct FluxProConfig {
    /// Delivery retry policy for signals waiting for a compatible node.
    pub signal_retry_policy: SignalRetryPolicy,
}

impl FluxProConfig {
    /// Returns the default engine configuration.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Reserved database configuration; currently has no configurable fields.
#[derive(Clone)]
pub struct FluxProDbConfig {}

impl FluxProDbConfig {
    /// Creates the reserved, empty database configuration.
    pub fn new() -> Self {
        Self {}
    }
}
