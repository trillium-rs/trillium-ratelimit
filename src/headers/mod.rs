//! Parse-and-format types for the IETF `RateLimit` / `RateLimit-Policy` HTTP header fields.
//!
//! These mirror the house style of trillium-forwarding's `Forwarded`: borrowed types over the
//! header bytes, with `from_headers` / `parse` / `Display` / `into_owned`. They carry no
//! dependency on the limiter handler, so a rate-limit-aware client can parse them without pulling
//! in the moka-backed machinery.

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
