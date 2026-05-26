//! Token-bucket arithmetic — the limiter's core algorithm, kept pure (the clock is injected) so
//! it is deterministically testable.

use crate::Quota;
use std::time::{Duration, Instant};

/// The token-bucket state for one partition: the current (fractional) token level and the instant
/// it was last refilled.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bucket {
    tokens: f64,
    last_update: Instant,
}

/// The outcome of attempting to consume from a [`Bucket`].
///
/// This reports raw facts about the bucket; mapping them onto the `RateLimit` header's `r` / `t`
/// parameters and `Retry-After` is the handler's responsibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Decision {
    /// Whether the request is admitted.
    pub allowed: bool,
    /// Whole tokens remaining after the attempt.
    pub remaining: u64,
    /// Time until the bucket would refill to its full burst capacity.
    pub time_to_full: Duration,
    /// When denied, the time until enough tokens accrue to admit this request. `None` when allowed.
    pub retry_after: Option<Duration>,
}

impl Bucket {
    /// A bucket that starts full, at the quota's burst capacity.
    pub(crate) fn new(now: Instant, quota: Quota) -> Self {
        Self {
            tokens: quota.burst() as f64,
            last_update: now,
        }
    }

    /// Refills for elapsed time, then attempts to consume `cost` tokens.
    pub(crate) fn try_consume(&mut self, now: Instant, quota: Quota, cost: u64) -> Decision {
        let rate = refill_per_second(quota);
        let burst = quota.burst() as f64;

        let elapsed = now.saturating_duration_since(self.last_update).as_secs_f64();
        self.tokens = (self.tokens + elapsed * rate).min(burst);
        self.last_update = now;

        let cost = cost as f64;
        let allowed = self.tokens >= cost;
        if allowed {
            self.tokens -= cost;
        }

        let retry_after = if allowed {
            None
        } else {
            Some(seconds_to_refill(cost - self.tokens, rate))
        };

        Decision {
            allowed,
            remaining: self.tokens.floor() as u64,
            time_to_full: seconds_to_refill(burst - self.tokens, rate),
            retry_after,
        }
    }
}

fn refill_per_second(quota: Quota) -> f64 {
    quota.count() as f64 / quota.window().as_secs_f64()
}

fn seconds_to_refill(tokens_needed: f64, rate: f64) -> Duration {
    if tokens_needed <= 0.0 || rate <= 0.0 {
        Duration::ZERO
    } else {
        Duration::from_secs_f64(tokens_needed / rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 10 units per second, burst 10.
    fn quota() -> Quota {
        Quota::per_second(10)
    }

    #[test]
    fn starts_full_then_denies() {
        let t0 = Instant::now();
        let mut bucket = Bucket::new(t0, quota());

        for i in 0..10 {
            assert!(bucket.try_consume(t0, quota(), 1).allowed, "token {i}");
        }

        let denied = bucket.try_consume(t0, quota(), 1);
        assert!(!denied.allowed);
        assert_eq!(denied.remaining, 0);
        let retry = denied.retry_after.unwrap();
        assert!(retry > Duration::ZERO && retry <= Duration::from_millis(100));
    }

    #[test]
    fn refills_fully_over_one_window() {
        let t0 = Instant::now();
        let q = quota();
        let mut bucket = Bucket::new(t0, q);
        for _ in 0..10 {
            bucket.try_consume(t0, q, 1);
        }
        assert!(!bucket.try_consume(t0, q, 1).allowed);

        let later = bucket.try_consume(t0 + Duration::from_secs(1), q, 1);
        assert!(later.allowed);
        assert_eq!(later.remaining, 9);
    }

    #[test]
    fn refills_partially() {
        let t0 = Instant::now();
        let q = quota();
        let mut bucket = Bucket::new(t0, q);
        for _ in 0..10 {
            bucket.try_consume(t0, q, 1);
        }

        let half = bucket.try_consume(t0 + Duration::from_millis(500), q, 1);
        assert!(half.allowed);
        assert_eq!(half.remaining, 4); // refilled 5, consumed 1
    }

    #[test]
    fn refill_is_capped_at_burst() {
        let t0 = Instant::now();
        let q = Quota::per_second(10).allow_burst(20);
        let mut bucket = Bucket::new(t0, q);

        let after_idle = bucket.try_consume(t0 + Duration::from_secs(100), q, 1);
        assert_eq!(after_idle.remaining, 19); // capped at 20, consumed 1
    }

    #[test]
    fn cost_greater_than_one() {
        let t0 = Instant::now();
        let q = quota();
        let mut bucket = Bucket::new(t0, q);

        let five = bucket.try_consume(t0, q, 5);
        assert!(five.allowed);
        assert_eq!(five.remaining, 5);

        let six = bucket.try_consume(t0, q, 6);
        assert!(!six.allowed);
        assert_eq!(six.remaining, 5);
    }
}
