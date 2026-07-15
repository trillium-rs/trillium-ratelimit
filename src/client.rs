//! Client-side rate limiting: a [`trillium-client`](https://docs.rs/trillium-client) handler that
//! paces outbound requests to stay under a per-origin [`Quota`], sleeping when necessary rather
//! than failing.
//!
//! Where the server-side [`RateLimiter`](crate::RateLimiter) *enforces* a quota against callers,
//! [`Throttle`] *respects* one as a caller — the polite-guest side of the same coin. Its motivating
//! use is talking to an API whose fixed request rate isn't advertised in headers (a crawler
//! keeping under a host's documented limit), where the job is simply "don't go faster than N per
//! window."
//!
//! ```no_run
//! use std::time::Duration;
//! use trillium_client::Client;
//! use trillium_ratelimit::{Quota, client::Throttle};
//! use trillium_testing::client_config;
//!
//! // At most 1 request/second to any single origin.
//! let client = Client::new(client_config()).with_handler(Throttle::new(Quota::per_second(1)));
//! ```

use crate::{Quota, RateLimit};
use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};
use trillium_client::{ClientHandler, Conn, KnownHeaderName::RetryAfter, Result, Url};

/// How a request's origin is reduced to the key its pacing budget is tracked under.
type Scope = Arc<dyn Fn(&Url) -> String + Send + Sync>;

/// A [`ClientHandler`] that paces outbound requests to stay under a [`Quota`], sleeping until each
/// request's turn rather than rejecting it.
///
/// Requests are metered per **scope** — by default each origin (scheme + host + port) gets its own
/// budget, so a client talking to several hosts limits each independently. Override the grouping
/// with [`with_scope`](Throttle::with_scope) to, for example, treat every subdomain of a host as
/// one budget.
///
/// # Pacing
///
/// The budget is a token bucket expressed as its virtual-scheduling (GCRA) dual: requests are
/// spaced `window / count` apart at saturation, with up to `burst` allowed back-to-back after an
/// idle period (see [`Quota::allow_burst`]). Concurrent requests to the same scope are admitted in
/// the order they reach the handler, each sleeping until its slot — so ordering is preserved
/// without an explicit queue.
///
/// # Server signals
///
/// By default [`Throttle`] also honors a server that pushes back: a `Retry-After` header, or a
/// `RateLimit` header reporting the scope exhausted (`r=0`), delays that scope's next request
/// accordingly. The configured [`Quota`] is the floor; these signals can only slow it further.
/// Disable with [`without_server_signals`](Throttle::without_server_signals).
///
/// This never retries or times out — layer a retry or timeout handler on top if you want those.
#[derive(Clone)]
pub struct Throttle {
    quota: Quota,
    interval: Duration,
    tolerance: Duration,
    scope: Scope,
    honor_server_signals: bool,
    state: Arc<Mutex<HashMap<String, Instant>>>,
}

impl Throttle {
    /// Builds a throttle enforcing `quota`, keyed per origin.
    #[must_use]
    pub fn new(quota: Quota) -> Self {
        let count = quota.count().max(1);
        let interval = Duration::from_secs_f64(quota.window().as_secs_f64() / count as f64);
        let burst_headroom = u32::try_from(quota.burst().saturating_sub(1)).unwrap_or(u32::MAX);
        let tolerance = interval
            .checked_mul(burst_headroom)
            .unwrap_or(Duration::MAX);
        Self {
            quota,
            interval,
            tolerance,
            scope: Arc::new(origin_scope),
            honor_server_signals: true,
            state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Replaces the scope function — the map from a request's URL to the key its budget is tracked
    /// under. The default keys per origin; supply a closure to group differently.
    ///
    /// ```
    /// use trillium_ratelimit::{Quota, client::Throttle};
    /// use trillium_client::Url;
    ///
    /// // Treat every subdomain of a registrable domain as one budget.
    /// let throttle = Throttle::new(Quota::per_second(1)).with_scope(|url: &Url| {
    ///     let host = url.host_str().unwrap_or_default();
    ///     host.rsplit('.').take(2).collect::<Vec<_>>().join(".")
    /// });
    /// ```
    #[must_use]
    pub fn with_scope(mut self, scope: impl Fn(&Url) -> String + Send + Sync + 'static) -> Self {
        self.scope = Arc::new(scope);
        self
    }

    /// Stops honoring server `Retry-After` / `RateLimit` push-back, pacing solely by the configured
    /// [`Quota`]. On by default.
    #[must_use]
    pub fn without_server_signals(mut self) -> Self {
        self.honor_server_signals = false;
        self
    }

    /// The quota this throttle paces to.
    #[must_use]
    pub fn quota(&self) -> Quota {
        self.quota
    }

    /// Claims the next slot for `scope` and returns how long to sleep before proceeding.
    ///
    /// The whole computation runs under the lock, but it neither awaits nor blocks — slot claims
    /// are strictly increasing, so requests wake in the order they claimed, and the sleep itself
    /// happens after the guard is dropped.
    fn claim(&self, scope: String, now: Instant) -> Duration {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let tat = state.entry(scope).or_insert(now);
        // May proceed once we are within `tolerance` of the theoretical arrival time; the schedule
        // itself always advances from `max(now, tat)`, so idle time is not banked beyond `tolerance`.
        let proceed_at = tat
            .checked_sub(self.tolerance)
            .map_or(now, |earliest| earliest.max(now));
        *tat = (*tat).max(now).checked_add(self.interval).unwrap_or(*tat);
        proceed_at.saturating_duration_since(now)
    }

    /// Pushes `scope`'s next-request time out by at least `backoff` from now, honoring a server's
    /// explicit request to slow down without letting burst headroom shorten it.
    fn back_off(&self, scope: String, backoff: Duration) {
        let target = Instant::now() + backoff + self.tolerance;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let tat = state.entry(scope).or_insert(target);
        *tat = (*tat).max(target);
    }
}

impl ClientHandler for Throttle {
    async fn run(&self, conn: &mut Conn) -> Result<()> {
        let delay = self.claim((self.scope)(conn.url()), Instant::now());
        if !delay.is_zero() {
            conn.client().connector().runtime().delay(delay).await;
        }
        Ok(())
    }

    async fn after_response(&self, conn: &mut Conn) -> Result<()> {
        if self.honor_server_signals
            && let Some(backoff) = server_backoff(conn)
        {
            self.back_off((self.scope)(conn.url()), backoff);
        }
        Ok(())
    }

    fn name(&self) -> std::borrow::Cow<'static, str> {
        "Throttle".into()
    }
}

/// The default scope: an origin's ASCII serialization, e.g. `https://docs.rs`.
fn origin_scope(url: &Url) -> String {
    url.origin().ascii_serialization()
}

/// The delay a server is asking us to wait before its next request, taken as the longest of a
/// delta-seconds `Retry-After` and any exhausted (`r=0`) `RateLimit` item's reset window. `None`
/// if the response carries neither signal.
fn server_backoff(conn: &Conn) -> Option<Duration> {
    let retry_after = conn
        .response_headers()
        .get_str(RetryAfter)
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs);

    let rate_limit_reset = RateLimit::from_headers(conn.response_headers())
        .unwrap_or_default()
        .into_iter()
        .filter(|limit| limit.remaining() == 0)
        .filter_map(|limit| limit.reset())
        .max();

    retry_after.into_iter().chain(rate_limit_reset).max()
}

impl fmt::Debug for Throttle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Throttle")
            .field("quota", &self.quota)
            .field("honor_server_signals", &self.honor_server_signals)
            .finish_non_exhaustive()
    }
}
