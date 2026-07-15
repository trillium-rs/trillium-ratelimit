//! Rate limiting for the [Trillium](https://trillium.rs) web framework.
//!
//! Two layers share one vocabulary, taken from the IETF [RateLimit header fields draft][draft]:
//!
//! - **The limiter** — a token-bucket `RateLimiter` handler that meters requests per partition
//!   key against a [`Quota`], halting over-quota requests and advertising `RateLimit` /
//!   `RateLimit-Policy` / `Retry-After` on metered responses. Behind the default `limiter`
//!   feature.
//! - **Header types** — [`RateLimit`] and [`RateLimitPolicy`] parse and format the corresponding
//!   HTTP fields. They are dependency-light and available without the `limiter` feature, so a
//!   rate-limit-aware client can parse what a server sends.
//! - **The client throttle** — behind the `client` feature, [`client::Throttle`] is a
//!   [`trillium-client`](https://docs.rs/trillium-client) handler that paces *outbound* requests
//!   to stay under a per-origin [`Quota`], sleeping until each request's turn. The polite-guest
//!   dual of the limiter, for talking to an API without exceeding its rate.
//!
//! ```
//! use trillium_ratelimit::{Quota, RateLimiter};
//!
//! // 60 requests/minute, keyed on the client's network.
//! let app = (
//!     RateLimiter::by_network(Quota::per_minute(60)),
//!     |conn: trillium::Conn| async move { conn.ok("hello") },
//! );
//! ```
//!
//! [draft]: https://datatracker.ietf.org/doc/draft-ietf-httpapi-ratelimit-headers/
#![forbid(unsafe_code)]
#![deny(
    clippy::dbg_macro,
    missing_copy_implementations,
    rustdoc::missing_crate_level_docs,
    missing_debug_implementations,
    missing_docs,
    nonstandard_style,
    unused_qualifications
)]

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
mod readme {}

pub mod headers;

#[doc(inline)]
pub use headers::{ParseError, Quota, QuotaUnit, RateLimit, RateLimitPolicy};

#[cfg(feature = "limiter")]
mod bucket;
#[cfg(feature = "limiter")]
mod limiter;
#[cfg(feature = "limiter")]
mod store;

#[cfg(feature = "limiter")]
pub use limiter::{MissingKey, RateLimiter};

#[cfg(feature = "client")]
pub mod client;
