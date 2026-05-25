# trillium-ratelimit

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

Rate limiting for the [Trillium](https://trillium.rs) web framework: a handler that meters
requests per partition key against a quota, plus standalone parse-and-format types for the
IETF `RateLimit` / `RateLimit-Policy` HTTP header fields. The header types carry no heavy
dependencies and are usable without the limiter — for example, by a rate-limit-aware client
retry handler.

## Example

```rust
// Replace with a real example once the public API lands.
```

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
