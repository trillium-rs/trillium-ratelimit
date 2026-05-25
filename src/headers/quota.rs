use std::{borrow::Cow, time::Duration};

/// A request quota: the number of [quota units](QuotaUnit) a partition may consume per time
/// window, and the maximum burst it may consume at once.
///
/// This maps directly onto the `RateLimit-Policy` header's `q` (quota) and `w` (window)
/// parameters. The token-bucket limiter derives its sustained refill rate from `count / window`
/// and its bucket capacity from `burst`.
///
/// # Examples
///
/// ```
/// use trillium_ratelimit::Quota;
/// use std::time::Duration;
///
/// let quota = Quota::per_minute(100);
/// assert_eq!(quota.count(), 100);
/// assert_eq!(quota.window(), Duration::from_secs(60));
/// assert_eq!(quota.burst(), 100); // burst defaults to the quota count
///
/// let bursty = Quota::per_second(10).allow_burst(50);
/// assert_eq!(bursty.burst(), 50);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    count: u64,
    window: Duration,
    burst: u64,
}

impl Quota {
    /// `count` quota units per second.
    pub fn per_second(count: u64) -> Self {
        Self::per(count, Duration::from_secs(1))
    }

    /// `count` quota units per minute.
    pub fn per_minute(count: u64) -> Self {
        Self::per(count, Duration::from_secs(60))
    }

    /// `count` quota units per hour.
    pub fn per_hour(count: u64) -> Self {
        Self::per(count, Duration::from_secs(60 * 60))
    }

    /// `count` quota units per arbitrary `window`.
    ///
    /// Burst capacity defaults to `count`; raise it with [`Quota::allow_burst`]. The `window`
    /// should be non-zero — the RFC requires `w` to be a positive integer number of seconds, and
    /// a zero window has no meaningful refill rate.
    pub fn per(count: u64, window: Duration) -> Self {
        Self {
            count,
            window,
            burst: count,
        }
    }

    /// Sets the maximum burst — the most quota units a partition may consume in an instant before
    /// being held to the sustained rate. Defaults to the quota count.
    pub fn allow_burst(mut self, burst: u64) -> Self {
        self.burst = burst;
        self
    }

    /// The number of quota units allowed per [`window`](Quota::window).
    pub fn count(&self) -> u64 {
        self.count
    }

    /// The time window over which [`count`](Quota::count) units are replenished.
    pub fn window(&self) -> Duration {
        self.window
    }

    /// The maximum burst capacity.
    pub fn burst(&self) -> u64 {
        self.burst
    }
}

/// The unit a [`Quota`] is measured in — the `RateLimit-Policy` `qu` parameter.
///
/// The v1 limiter only enforces [`QuotaUnit::Requests`]. The other variants exist so the header
/// types can faithfully represent and round-trip policies advertised by other servers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum QuotaUnit<'a> {
    /// `requests` — one unit per request. The default when `qu` is absent.
    #[default]
    Requests,

    /// `content-bytes` — units measured in content bytes.
    ContentBytes,

    /// `concurrent-requests` — units measured in concurrently-open requests.
    ConcurrentRequests,

    /// A unit from the IANA RateLimit Quota Units registry not known to this crate.
    Other(Cow<'a, str>),
}

impl<'a> QuotaUnit<'a> {
    /// The wire token for this unit, e.g. `"requests"`.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Requests => "requests",
            Self::ContentBytes => "content-bytes",
            Self::ConcurrentRequests => "concurrent-requests",
            Self::Other(other) => other,
        }
    }

    /// Parses a unit from its wire token, recognizing the three registered units and capturing
    /// anything else as [`QuotaUnit::Other`].
    pub fn from_token(token: &'a str) -> Self {
        Self::from_cow(Cow::Borrowed(token))
    }

    pub(crate) fn from_cow(value: Cow<'a, str>) -> Self {
        match value.as_ref() {
            "requests" => Self::Requests,
            "content-bytes" => Self::ContentBytes,
            "concurrent-requests" => Self::ConcurrentRequests,
            _ => Self::Other(value),
        }
    }

    /// Converts a borrowed `QuotaUnit` into an owned `QuotaUnit<'static>`.
    pub fn into_owned(self) -> QuotaUnit<'static> {
        match self {
            Self::Requests => QuotaUnit::Requests,
            Self::ContentBytes => QuotaUnit::ContentBytes,
            Self::ConcurrentRequests => QuotaUnit::ConcurrentRequests,
            Self::Other(other) => QuotaUnit::Other(Cow::Owned(other.into_owned())),
        }
    }
}
