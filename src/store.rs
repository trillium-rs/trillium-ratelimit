//! The in-memory, concurrent per-partition bucket store, backed by moka.
//!
//! The `consume` signature is the seam a future distributed backend would reimplement as a single
//! atomic round-trip.

use crate::{
    Quota,
    bucket::{Bucket, Decision},
};
use moka::{ops::compute::Op, sync::Cache};
use std::{hash::Hash, time::Instant};

/// A concurrent map of partition key to token [`Bucket`], with size-bounded TinyLFU eviction and
/// idle expiry tuned to the quota window.
pub(crate) struct Store<K> {
    cache: Cache<K, Bucket>,
    quota: Quota,
}

impl<K> Store<K>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
{
    /// Builds a store for `quota`, holding at most `max_partitions` buckets.
    ///
    /// Idle buckets expire after one window — a bucket untouched for that long would have refilled
    /// to full anyway, so dropping it is lossless. The capacity cap is the memory backstop against
    /// a high-cardinality flood; TinyLFU keeps the hot (abusive) keys and evicts one-shot ones.
    pub(crate) fn new(quota: Quota, max_partitions: u64) -> Self {
        let cache = Cache::builder()
            .max_capacity(max_partitions)
            .time_to_idle(quota.window())
            .build();
        Self { cache, quota }
    }

    /// Atomically refills and consumes `cost` from `key`'s bucket, returning the [`Decision`].
    ///
    /// The read-modify-write runs inside moka's per-key compute lock, so concurrent calls on the
    /// same key cannot over-admit.
    pub(crate) fn consume(&self, key: K, cost: u64) -> Decision {
        let quota = self.quota;
        let now = Instant::now();
        let mut decision = None;

        self.cache.entry(key).and_compute_with(|entry| {
            let mut bucket = entry
                .map(|entry| entry.into_value())
                .unwrap_or_else(|| Bucket::new(now, quota));
            decision = Some(bucket.try_consume(now, quota, cost));
            Op::Put(bucket)
        });

        decision.expect("and_compute_with always runs the compute closure")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
        thread,
    };

    #[test]
    fn concurrent_consume_admits_exactly_burst() {
        // A long window makes refill negligible over the test's runtime, so the admitted count is
        // exactly the initial burst (100) regardless of scheduling.
        let store = Arc::new(Store::new(Quota::per_hour(100), 1000));
        let admitted = Arc::new(AtomicU64::new(0));

        let threads: Vec<_> = (0..16)
            .map(|_| {
                let store = Arc::clone(&store);
                let admitted = Arc::clone(&admitted);
                thread::spawn(move || {
                    for _ in 0..50 {
                        if store.consume("shared-key", 1).allowed {
                            admitted.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                })
            })
            .collect();

        for thread in threads {
            thread.join().unwrap();
        }

        assert_eq!(admitted.load(Ordering::Relaxed), 100);
    }

    #[test]
    fn distinct_keys_have_independent_buckets() {
        let store = Store::new(Quota::per_hour(1), 1000);
        assert!(store.consume("a", 1).allowed);
        assert!(store.consume("b", 1).allowed);
        assert!(!store.consume("a", 1).allowed); // a is now exhausted
        assert!(!store.consume("b", 1).allowed);
    }
}
