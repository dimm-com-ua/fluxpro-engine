use chrono::{DateTime, Duration, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalRetryTier {
    pub name: &'static str,
    pub until: Duration,
    pub delay: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalRetryDecision {
    pub phase: &'static str,
    pub retry_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub age: Duration,
    pub delay: Duration,
}

/// Durable delivery policy for signals that arrive before their accepting node.
///
/// The default horizon is intentionally longer than the longest 24-hour waits
/// used by the TK Online and Cash Online process definitions.
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
    pub fn new(tiers: Vec<SignalRetryTier>, expires_after: Duration) -> Self {
        Self {
            tiers,
            expires_after,
        }
    }

    pub fn expires_after(&self) -> Duration {
        self.expires_after
    }

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
