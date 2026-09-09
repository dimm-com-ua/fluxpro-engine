//! Age-based retry decisions for signals awaiting a compatible node.

use chrono::{DateTime, Duration, Utc};

/// One age range in a signal retry policy; tiers are checked in supplied order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalRetryTier {
    /// Stable tier name used in delivery diagnostics.
    pub name: &'static str,
    /// Exclusive maximum signal age covered by this tier.
    pub until: Duration,
    /// Delay before the next delivery attempt.
    pub delay: Duration,
}

/// Next delivery time and diagnostic metadata for a deferred signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalRetryDecision {
    /// Name of the selected retry tier.
    pub phase: &'static str,
    /// UTC time for the next signal delivery attempt.
    pub retry_at: DateTime<Utc>,
    /// UTC deadline beyond which delivery is no longer retried.
    pub expires_at: DateTime<Utc>,
    /// Elapsed time since admission, clamped to zero for future timestamps.
    pub age: Duration,
    /// Delay before the next delivery attempt.
    pub delay: Duration,
}

/// Durable delivery policy for signals that arrive before their accepting node.
///
/// The default policy retries for up to 26 hours, with increasingly spaced
/// attempts so signals can survive long-running waits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalRetryPolicy {
    tiers: Vec<SignalRetryTier>,
    expires_after: Duration,
}

impl Default for SignalRetryPolicy {
    fn default() -> Self {
        Self {
            tiers: vec![
                SignalRetryTier {
                    name: "immediate",
                    until: Duration::seconds(10),
                    delay: Duration::milliseconds(250),
                },
                SignalRetryTier {
                    name: "short",
                    until: Duration::minutes(3),
                    delay: Duration::seconds(3),
                },
                SignalRetryTier {
                    name: "medium",
                    until: Duration::minutes(13),
                    delay: Duration::seconds(30),
                },
                SignalRetryTier {
                    name: "long",
                    until: Duration::hours(1),
                    delay: Duration::minutes(2),
                },
                SignalRetryTier {
                    name: "extended",
                    until: Duration::hours(26),
                    delay: Duration::minutes(5),
                },
            ],
            expires_after: Duration::hours(26),
        }
    }
}

impl SignalRetryPolicy {
    /// Creates a policy from ordered tiers and an expiry horizon.
    ///
    /// The constructor does not validate ordering, positive delays, or tier coverage.
    pub fn new(tiers: Vec<SignalRetryTier>, expires_after: Duration) -> Self {
        Self {
            tiers,
            expires_after,
        }
    }

    /// Returns the maximum age of a signal eligible for deferred delivery.
    pub fn expires_after(&self) -> Duration {
        self.expires_after
    }

    /// Selects the first tier whose age boundary exceeds the signal age.
    ///
    /// Returns `None` when the signal has expired or no tier covers its age.
    pub fn decision(
        &self,
        queued_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Option<SignalRetryDecision> {
        let age = now.signed_duration_since(queued_at).max(Duration::zero());
        if age >= self.expires_after {
            return None;
        }
        let tier = self.tiers.iter().find(|tier| age < tier.until)?;
        Some(SignalRetryDecision {
            phase: tier.name,
            retry_at: now + tier.delay,
            expires_at: queued_at + self.expires_after,
            age,
            delay: tier.delay,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_moves_through_backoff_tiers() {
        let policy = SignalRetryPolicy::default();
        let queued_at = Utc::now();

        assert_eq!(
            policy.decision(queued_at, queued_at).unwrap().phase,
            "immediate"
        );
        assert_eq!(
            policy
                .decision(queued_at, queued_at + Duration::seconds(11))
                .unwrap()
                .phase,
            "short"
        );
        assert_eq!(
            policy
                .decision(queued_at, queued_at + Duration::minutes(4))
                .unwrap()
                .phase,
            "medium"
        );
        assert_eq!(
            policy
                .decision(queued_at, queued_at + Duration::hours(2))
                .unwrap()
                .phase,
            "extended"
        );
    }

    #[test]
    fn default_policy_expires_after_twenty_six_hours() {
        let policy = SignalRetryPolicy::default();
        let queued_at = Utc::now();

        assert!(
            policy
                .decision(queued_at, queued_at + Duration::hours(26))
                .is_none()
        );
    }
}
