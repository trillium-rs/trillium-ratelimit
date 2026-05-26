//! Parse-and-format types for the IETF [RateLimit header fields draft][draft]'s `RateLimit` and
//! `RateLimit-Policy` HTTP fields, which are [RFC 9651] Structured Field Lists.
//!
//! [`RateLimit`] and [`RateLimitPolicy`] are borrowed types over the header bytes, exposing
//! `from_headers` / `parse_list` / [`Display`](std::fmt::Display) / `into_owned`. They are
//! dependency-light and available without the `limiter` feature, so a rate-limit-aware client can
//! parse what a server sends.
//!
//! [draft]: https://datatracker.ietf.org/doc/draft-ietf-httpapi-ratelimit-headers/
//! [RFC 9651]: https://www.rfc-editor.org/rfc/rfc9651

mod quota;
mod rate_limit;
mod rate_limit_policy;
mod sf;

pub use quota::{Quota, QuotaUnit};
pub use rate_limit::RateLimit;
pub use rate_limit_policy::RateLimitPolicy;

use std::fmt;

/// The error returned when a `RateLimit` / `RateLimit-Policy` header value is not well-formed as
/// an RFC 9651 Structured Field. Per the RateLimit draft, a caller that receives this should
/// treat the field as carrying no rate-limit information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError;

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("malformed RateLimit structured field")
    }
}

impl std::error::Error for ParseError {}
