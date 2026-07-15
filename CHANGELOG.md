# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.0.2] - 2026-07-15

### Added

- `client` feature: `client::Throttle`, a `trillium-client` handler that paces outbound requests
  to stay under a per-origin `Quota`, sleeping until each request's turn rather than rejecting.
  Origin-scoped by default (overridable via `with_scope`); honors server `Retry-After` /
  exhausted `RateLimit` push-back by default (`without_server_signals` to disable).

## [0.0.1](https://github.com/trillium-rs/trillium-ratelimit/compare/v0.0.0...v0.0.1) - 2026-07-13

### Added

- unmap dual-stack ip addrs

### Other

- fmt
