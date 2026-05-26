# 🚦 trillium-ratelimit — rate limiting and RateLimit header types

[![ci][ci-badge]][ci]
[![crates.io version][version-badge]][crate]
[![docs.rs][docs-badge]][docs]
[![codecov][codecov-badge]][codecov]

[ci]: https://github.com/trillium-rs/trillium-ratelimit/actions?query=workflow%3ACI
[ci-badge]: https://github.com/trillium-rs/trillium-ratelimit/workflows/CI/badge.svg
[version-badge]: https://img.shields.io/crates/v/trillium-ratelimit.svg?style=flat-square
[crate]: https://crates.io/crates/trillium-ratelimit
[docs-badge]: https://img.shields.io/badge/docs-latest-blue.svg?style=flat-square
[docs]: https://docs.rs/trillium-ratelimit
[codecov-badge]: https://codecov.io/gh/trillium-rs/trillium-ratelimit/graph/badge.svg
[codecov]: https://codecov.io/gh/trillium-rs/trillium-ratelimit

Rate limiting for the [Trillium](https://trillium.rs) web framework: a token-bucket handler
that meters requests per partition key against a quota, plus standalone parse-and-format types
for the IETF `RateLimit` / `RateLimit-Policy` HTTP header fields.

The handler guards expensive or unauthenticated endpoints and enforces per-principal quotas; it
advertises `RateLimit` / `RateLimit-Policy` / `Retry-After` on every metered response. The
header types are dependency-light and usable on their own — disable default features to depend
only on them, as a rate-limit-aware client would to parse what a server sends.

## Example

```rust
use trillium_ratelimit::{Quota, RateLimiter};

// 60 requests/minute, keyed on the client's network — a guard for an unauthenticated endpoint.
let app = (
    RateLimiter::by_network(Quota::per_minute(60)),
    |conn: trillium::Conn| async move { conn.ok("hello") },
);

// run with your chosen runtime adapter, e.g.:
// trillium_tokio::run(app);
```

Stack several limiters to enforce overlapping scopes — each appends its own item to the
response headers. Key on a value an upstream handler placed in state (an authenticated user or
API-key id) by passing a closure to `RateLimiter::new`, or use `RateLimiter::from_state` when
the state value is itself the key.

## Safety

This crate uses `#![forbid(unsafe_code)]`.

## License

<sup>
Licensed under either of <a href="LICENSE-APACHE">Apache License, Version
2.0</a> or <a href="LICENSE-MIT">MIT license</a> at your option.
</sup>

<br/>

<sub>
Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
</sub>
