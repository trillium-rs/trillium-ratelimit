//! Rate limiting for the [Trillium](https://trillium.rs) web framework.
//!
//! This crate has two layers that share one vocabulary, drawn from the IETF
//! `RateLimit` header fields draft:
//!
//! - **Header types** — parse-and-format types for the `RateLimit` and `RateLimit-Policy`
//!   HTTP fields. These carry no heavy dependencies and are useful on their own: a server
//!   that rolls its own limiter can format them, and a rate-limit-aware client retry handler
//!   can parse them. Always available.
//! - **The limiter** — a trillium `Handler` that meters requests per partition key against a
//!   quota, backed by an in-memory store. Behind the default `limiter` feature; disable
//!   default features to depend only on the header types.
//!
//! See `PLAN.md` in the repository for the v1 design and roadmap. The public API is still
//! being built out.
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
