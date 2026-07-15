use crate::{Quota, RateLimit, RateLimitPolicy, store::Store};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    fmt,
    hash::Hash,
    net::{IpAddr, Ipv6Addr},
    time::Duration,
};
use trillium::{Conn, Handler, Status};

const DEFAULT_MAX_PARTITIONS: u64 = 100_000;
const DEFAULT_POLICY_NAME: &str = "default";

/// What to do when the key extractor returns `None` — the request carries no partition key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MissingKey {
    /// Pass the request through unmetered. The default: a missing key is an authentication
    /// concern, not the limiter's. This suits an identity-keyed limiter placed after a loader —
    /// un-identified requests fall through to whatever comes next (often the auth gate itself).
    #[default]
    Skip,
    /// Reject the request with the configured status, consuming no quota.
    Reject,
    /// Meter every keyless request against a single shared bucket.
    Shared,
}

type Extractor<K> = Box<dyn Fn(&Conn) -> Option<K> + Send + Sync>;
type PartitionKeyRenderer<K> = Box<dyn Fn(&K) -> Vec<u8> + Send + Sync>;

/// A token-bucket rate-limiting [`Handler`] that meters requests per partition key against a
/// [`Quota`].
///
/// Place it ahead of whatever it should guard; because trillium has no middleware/endpoint
/// split, "which routes" is answered by where you mount it. An allowed request passes through
/// (annotated with `RateLimit` / `RateLimit-Policy` headers); an over-quota request is halted
/// with the configured status (429 by default) plus `Retry-After`.
///
/// The partition key comes from a closure you supply — typically reading a value an upstream
/// handler stored in conn state. Returning `None` from it triggers the [`MissingKey`] policy
/// ([`Skip`](MissingKey::Skip) by default). Stack several limiters to enforce overlapping scopes;
/// each appends its own policy to the response headers.
///
/// ```
/// use trillium_ratelimit::{Quota, RateLimiter};
/// use trillium::Conn;
///
/// # #[derive(Clone, Hash, PartialEq, Eq)]
/// # struct UserId(u64);
/// // 100 requests/minute, keyed by a `UserId` an upstream handler placed in state:
/// let _limiter = RateLimiter::new(Quota::per_minute(100), |conn: &Conn| {
///     conn.state::<UserId>().cloned()
/// });
/// ```
pub struct RateLimiter<K> {
    store: Store<Option<K>>,
    extractor: Extractor<K>,
    quota: Quota,
    policy_name: String,
    missing_key: MissingKey,
    status: Status,
    jitter: Duration,
    partition_key: Option<PartitionKeyRenderer<K>>,
}

impl<K> RateLimiter<K>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
{
    /// Builds a rate limiter enforcing `quota`, deriving each request's partition key from
    /// `extractor`. Defaults: policy name `"default"`, [`MissingKey::Skip`], status 429.
    pub fn new(
        quota: Quota,
        extractor: impl Fn(&Conn) -> Option<K> + Send + Sync + 'static,
    ) -> Self {
        Self {
            store: Store::new(quota, DEFAULT_MAX_PARTITIONS),
            extractor: Box::new(extractor),
            quota,
            policy_name: DEFAULT_POLICY_NAME.to_string(),
            missing_key: MissingKey::Skip,
            status: Status::TooManyRequests,
            jitter: Duration::ZERO,
            partition_key: None,
        }
    }

    /// Builds a rate limiter keyed on a value an upstream handler placed in conn state.
    ///
    /// Sugar for [`new`](RateLimiter::new) with an extractor that clones the `K` out of conn
    /// state, applying the [`MissingKey`] policy when no such value is present. Use it when the
    /// state value *is* the key (a `UserId` newtype); for a field of a larger state value, write
    /// the extractor with [`new`](RateLimiter::new).
    ///
    /// ```
    /// use trillium_ratelimit::{Quota, RateLimiter};
    /// # #[derive(Clone, Hash, PartialEq, Eq)]
    /// # struct ApiKeyId(u64);
    /// let _limiter = RateLimiter::<ApiKeyId>::from_state(Quota::per_hour(10_000));
    /// ```
    pub fn from_state(quota: Quota) -> Self {
        Self::new(quota, |conn: &Conn| conn.state::<K>().cloned())
    }

    /// Sets the policy name reported in the `RateLimit` / `RateLimit-Policy` headers. Give stacked
    /// limiters distinct names so their policies are individually identifiable.
    pub fn with_policy_name(mut self, policy_name: impl Into<String>) -> Self {
        self.policy_name = policy_name.into();
        self
    }

    /// Sets the policy applied when the extractor returns `None`. Defaults to [`MissingKey::Skip`].
    pub fn with_missing_key(mut self, missing_key: MissingKey) -> Self {
        self.missing_key = missing_key;
        self
    }

    /// Sets the response status used when a request is rejected. Defaults to 429 Too Many Requests.
    pub fn with_status(mut self, status: Status) -> Self {
        self.status = status;
        self
    }

    /// Sets the maximum number of partitions held in memory at once. Defaults to 100,000.
    ///
    /// This is the memory backstop against a high-cardinality flood: once the cap is reached,
    /// least-valuable buckets are evicted (an evicted key simply reconstructs as full on its next
    /// request). Raise it for many concurrent legitimate partitions; lower it under tight memory.
    pub fn with_capacity(mut self, max_partitions: u64) -> Self {
        self.store = Store::new(self.quota, max_partitions);
        self
    }

    /// Adds up to `jitter` of random delay to the advertised reset (`t`) and `Retry-After`.
    ///
    /// Without jitter, every client throttled in the same instant is told to come back at the same
    /// instant, producing a thundering herd at the window edge. A random spread over `[0, jitter]`
    /// smears those retries out. Defaults to no jitter.
    pub fn with_jitter(mut self, jitter: Duration) -> Self {
        self.jitter = jitter;
        self
    }

    /// Emits the partition key on the wire (the `pk` parameter), rendering each key to bytes with
    /// `render`. Off by default.
    ///
    /// The bytes are base64-encoded into the Structured Field Byte Sequence the draft requires.
    /// Emission is opt-in because a partition key can carry identifying information about the
    /// client; render only request-derived, non-sensitive bytes (a hashed or opaque form of the
    /// key rather than a raw user id). The key is only emitted when the request actually carries
    /// one — a [`MissingKey::Shared`] keyless request advertises no `pk`.
    pub fn with_partition_key(
        mut self,
        render: impl Fn(&K) -> Vec<u8> + Send + Sync + 'static,
    ) -> Self {
        self.partition_key = Some(Box::new(render));
        self
    }

    /// A uniformly random delay in `[0, self.jitter]`, drawn once per request.
    fn random_jitter(&self) -> Duration {
        if self.jitter.is_zero() {
            Duration::ZERO
        } else {
            let max = u64::try_from(self.jitter.as_nanos()).unwrap_or(u64::MAX);
            Duration::from_nanos(fastrand::u64(0..=max))
        }
    }
}

impl RateLimiter<IpAddr> {
    /// Builds a rate limiter keyed on the client's network, derived from the connection's peer IP.
    ///
    /// Sugar for [`new`](RateLimiter::new). IPv4 peers are keyed on the full address; IPv6 peers
    /// are grouped by their `/64` prefix, since a single client is typically allocated a whole
    /// `/64`. A connection with no peer IP carries no key and so follows the [`MissingKey`] policy
    /// like any other absent key — behind a reverse proxy, recover the real client IP into the
    /// connection first (for example with [`trillium-forwarding`]) so it has one to key on.
    ///
    /// [`trillium-forwarding`]: https://docs.rs/trillium-forwarding
    ///
    /// ```
    /// use trillium_ratelimit::{Quota, RateLimiter};
    /// let _limiter = RateLimiter::by_network(Quota::per_minute(60));
    /// ```
    pub fn by_network(quota: Quota) -> Self {
        Self::new(quota, |conn| conn.peer_ip().map(network_key))
    }
}

/// Reduces a peer IP to its network key: IPv4 unchanged (`/32`), IPv6 masked to its `/64` prefix.
///
/// An IPv4-mapped address (`::ffff:a.b.c.d`) keys as the IPv4 peer it is. A dual-stack listener —
/// one bound to `::`, which is how a single socket serves both families — reports *every* IPv4
/// connection that way, and the mapped form carries its address in the low 32 bits. Masking it as
/// IPv6 would zero precisely the bits that distinguish one client from another, collapsing every
/// IPv4 peer on the internet onto a single `::` partition to share one bucket.
fn network_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(addr) => match addr.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let [a, b, c, d, ..] = addr.segments();
                IpAddr::V6(Ipv6Addr::new(a, b, c, d, 0, 0, 0, 0))
            }
        },
    }
}

impl<K> Handler for RateLimiter<K>
where
    K: Hash + Eq + Clone + Send + Sync + 'static,
{
    async fn run(&self, mut conn: Conn) -> Conn {
        let key = match (self.extractor)(&conn) {
            Some(key) => Some(key),
            None => match self.missing_key {
                MissingKey::Skip => return conn,
                MissingKey::Reject => return conn.with_status(self.status).halt(),
                MissingKey::Shared => None,
            },
        };

        let pk = match (&self.partition_key, &key) {
            (Some(render), Some(key)) => Some(STANDARD.encode(render(key))),
            _ => None,
        };

        let decision = self.store.consume(key, 1);
        let jitter = self.random_jitter();

        let mut policy = RateLimitPolicy::new(self.policy_name.as_str(), self.quota.count())
            .with_window(self.quota.window());
        let mut limit = RateLimit::new(self.policy_name.as_str(), decision.remaining).with_reset(
            Duration::from_secs(secs_ceil(decision.time_to_full + jitter)),
        );
        if let Some(pk) = &pk {
            policy = policy.with_partition_key(pk.clone());
            limit = limit.with_partition_key(pk.clone());
        }

        conn.response_headers_mut()
            .append("RateLimit-Policy", policy.to_string());
        conn.response_headers_mut()
            .append("RateLimit", limit.to_string());

        if decision.allowed {
            conn
        } else {
            let retry_after = secs_ceil(decision.retry_after.unwrap_or_default() + jitter);
            conn.response_headers_mut()
                .insert("Retry-After", retry_after.to_string());
            conn.with_status(self.status).halt()
        }
    }
}

impl<K> fmt::Debug for RateLimiter<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateLimiter")
            .field("quota", &self.quota)
            .field("policy_name", &self.policy_name)
            .field("missing_key", &self.missing_key)
            .field("status", &self.status)
            .field("jitter", &self.jitter)
            .finish_non_exhaustive()
    }
}

/// Round a sub-second duration up to whole seconds, so we never advertise a retry sooner than the
/// bucket can actually admit.
fn secs_ceil(duration: Duration) -> u64 {
    duration.as_secs() + u64::from(duration.subsec_nanos() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use trillium_testing::{TestServer, harness, test};

    fn keyed() -> impl Fn(&Conn) -> Option<&'static str> + Send + Sync {
        |_: &Conn| Some("shared-test-key")
    }

    fn keyless() -> impl Fn(&Conn) -> Option<&'static str> + Send + Sync {
        |_: &Conn| None
    }

    #[test(harness)]
    async fn allows_and_advertises_within_quota() {
        let app = TestServer::new((RateLimiter::new(Quota::per_minute(5), keyed()), "ok")).await;

        let conn = app.get("/").await;
        conn.assert_status(Status::Ok);
        conn.assert_body("ok");
        let policy = conn.header("RateLimit-Policy").expect("policy header");
        assert!(policy.contains("q=5"), "got {policy:?}");
        let limit = conn.header("RateLimit").expect("ratelimit header");
        assert!(limit.contains("r="), "got {limit:?}");
    }

    #[test(harness)]
    async fn denies_over_quota_with_retry_after() {
        let app = TestServer::new((RateLimiter::new(Quota::per_minute(1), keyed()), "ok")).await;

        app.get("/").await.assert_status(Status::Ok);

        let conn = app.get("/").await;
        conn.assert_status(Status::TooManyRequests);
        conn.assert_header("Retry-After", "60");
    }

    #[test(harness)]
    async fn skip_passes_keyless_through_unmetered() {
        let app = TestServer::new((RateLimiter::new(Quota::per_minute(1), keyless()), "ok")).await;

        // Default MissingKey::Skip: every keyless request passes, with no rate-limit headers.
        for _ in 0..3 {
            let conn = app.get("/").await;
            conn.assert_status(Status::Ok);
            conn.assert_no_header("RateLimit");
        }
    }

    #[test(harness)]
    async fn reject_blocks_keyless() {
        let app = TestServer::new((
            RateLimiter::new(Quota::per_minute(1), keyless()).with_missing_key(MissingKey::Reject),
            "ok",
        ))
        .await;

        app.get("/").await.assert_status(Status::TooManyRequests);
    }

    #[test(harness)]
    async fn shared_meters_keyless_together() {
        let app = TestServer::new((
            RateLimiter::new(Quota::per_minute(1), keyless()).with_missing_key(MissingKey::Shared),
            "ok",
        ))
        .await;

        app.get("/").await.assert_status(Status::Ok);
        app.get("/").await.assert_status(Status::TooManyRequests);
    }

    #[test(harness)]
    async fn custom_status() {
        let app = TestServer::new((
            RateLimiter::new(Quota::per_minute(1), keyed()).with_status(Status::ServiceUnavailable),
            "ok",
        ))
        .await;

        app.get("/").await.assert_status(Status::Ok);
        app.get("/").await.assert_status(Status::ServiceUnavailable);
    }

    #[derive(Clone, Hash, PartialEq, Eq)]
    struct UserId(u32);

    #[test(harness)]
    async fn from_state_keys_on_state_value() {
        use trillium::State;

        // An upstream handler placed the key in state, so the request is metered against it.
        let metered = TestServer::new((
            State::new(UserId(1)),
            RateLimiter::<UserId>::from_state(Quota::per_minute(1)),
            "ok",
        ))
        .await;
        metered.get("/").await.assert_status(Status::Ok);
        metered
            .get("/")
            .await
            .assert_status(Status::TooManyRequests);

        // With no such state value, the request carries no key, so MissingKey::Skip passes it.
        let keyless = TestServer::new((
            RateLimiter::<UserId>::from_state(Quota::per_minute(1)),
            "ok",
        ))
        .await;
        keyless.get("/").await.assert_status(Status::Ok);
        keyless.get("/").await.assert_status(Status::Ok);
    }

    #[test(harness)]
    async fn by_network_meters_per_peer_ip() {
        let app = TestServer::new((RateLimiter::by_network(Quota::per_minute(1)), "ok")).await;

        // `connection: close` forces a fresh connection per request: the test transport binds the
        // peer IP at connection time, so a pooled/reused connection would carry the previous peer's
        // IP and defeat the per-peer distinction this test checks.
        app.get("/")
            .with_peer_ip([10, 0, 0, 1])
            .with_request_header("connection", "close")
            .await
            .assert_status(Status::Ok);
        app.get("/")
            .with_peer_ip([10, 0, 0, 1])
            .with_request_header("connection", "close")
            .await
            .assert_status(Status::TooManyRequests);

        // A different address has its own bucket.
        app.get("/")
            .with_peer_ip([10, 0, 0, 2])
            .with_request_header("connection", "close")
            .await
            .assert_status(Status::Ok);

        // No peer IP (the default test transport) means no key, so Skip passes it through.
        app.get("/").await.assert_status(Status::Ok);
    }

    #[test]
    fn network_key_masks_ipv6_to_64() {
        use std::net::{Ipv4Addr, Ipv6Addr};

        // IPv4 is keyed on the whole address.
        let v4 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        assert_eq!(network_key(v4), v4);

        // Two IPv6 addresses sharing a /64 collapse to the same key; a different /64 does not.
        let a = "2001:db8:1:2:aaaa:bbbb:cccc:dddd"
            .parse::<Ipv6Addr>()
            .unwrap();
        let b = "2001:db8:1:2:1111:2222:3333:4444"
            .parse::<Ipv6Addr>()
            .unwrap();
        let c = "2001:db8:1:3::1".parse::<Ipv6Addr>().unwrap();
        assert_eq!(network_key(a.into()), network_key(b.into()));
        assert_ne!(network_key(a.into()), network_key(c.into()));
        assert_eq!(
            network_key(a.into()),
            IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 1, 2, 0, 0, 0, 0))
        );
    }

    /// A dual-stack listener (bound to `::`) reports every IPv4 peer as an IPv4-mapped IPv6
    /// address, whose distinguishing bits all live below the `/64` an IPv6 mask would keep. Keyed
    /// as IPv6 they would all collapse onto `::` and meter against one shared bucket — so the whole
    /// IPv4 internet, which is most real traffic, would throttle collectively.
    #[test]
    fn network_key_unmaps_ipv4_mapped_ipv6() {
        use std::net::Ipv4Addr;

        let mapped = "::ffff:198.51.100.1".parse::<IpAddr>().unwrap();
        assert_eq!(
            network_key(mapped),
            IpAddr::V4(Ipv4Addr::new(198, 51, 100, 1))
        );

        // Distinct IPv4 peers keep distinct buckets when they arrive mapped...
        let other = "::ffff:203.0.113.9".parse::<IpAddr>().unwrap();
        assert_ne!(network_key(mapped), network_key(other));

        // ...and a mapped peer keys identically to the same address arriving unmapped, so one
        // client cannot double its quota by reaching the same server over both families.
        assert_eq!(
            network_key(mapped),
            network_key(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 1)))
        );
    }

    #[test(harness)]
    async fn jitter_spreads_retry_after_within_bounds() {
        // Base retry for a depleted per-minute(1) bucket is 60s; jitter adds [0, 30].
        let app = TestServer::new((
            RateLimiter::new(Quota::per_minute(1), keyed()).with_jitter(Duration::from_secs(30)),
            "ok",
        ))
        .await;

        app.get("/").await.assert_status(Status::Ok);

        let mut seen = std::collections::HashSet::new();
        for _ in 0..30 {
            let conn = app.get("/").await;
            conn.assert_status(Status::TooManyRequests);
            let retry: u64 = conn.header("Retry-After").unwrap().parse().unwrap();
            assert!(
                (60..=91).contains(&retry),
                "retry-after {retry} out of bounds"
            );
            seen.insert(retry);
        }
        assert!(seen.len() > 1, "jitter produced no spread: {seen:?}");
    }

    #[test(harness)]
    async fn partition_key_emitted_when_configured() {
        let app = TestServer::new((
            RateLimiter::new(Quota::per_minute(5), keyed())
                .with_partition_key(|key: &&str| key.as_bytes().to_vec()),
            "ok",
        ))
        .await;

        let conn = app.get("/").await;
        let expected = STANDARD.encode("shared-test-key");
        let limit = RateLimit::from_headers(conn.response_headers()).unwrap();
        assert_eq!(limit[0].partition_key(), Some(expected.as_str()));
        let policy = RateLimitPolicy::from_headers(conn.response_headers()).unwrap();
        assert_eq!(policy[0].partition_key(), Some(expected.as_str()));
    }

    #[test(harness)]
    async fn stacked_limiters_each_append_their_policy() {
        let app = TestServer::new((
            RateLimiter::new(Quota::per_minute(5), keyed()).with_policy_name("per-minute"),
            RateLimiter::new(Quota::per_hour(100), keyed()).with_policy_name("per-hour"),
            "ok",
        ))
        .await;

        let conn = app.get("/").await;
        let policies = RateLimitPolicy::from_headers(conn.response_headers()).unwrap();
        let names: Vec<_> = policies.iter().map(|p| p.name()).collect();
        assert_eq!(names, ["per-minute", "per-hour"]);

        let limits = RateLimit::from_headers(conn.response_headers()).unwrap();
        let names: Vec<_> = limits.iter().map(|l| l.name()).collect();
        assert_eq!(names, ["per-minute", "per-hour"]);
    }
}
