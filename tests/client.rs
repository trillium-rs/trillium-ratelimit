//! End-to-end tests for the client-side [`Throttle`] handler over an in-process
//! [`ServerConnector`].

use std::{
    future::IntoFuture,
    time::{Duration, Instant},
};
use trillium_client::Client;
use trillium_ratelimit::{Quota, client::Throttle};
use trillium_testing::{
    ServerConnector, TestResult, futures_lite::future::zip, harness, prelude::Conn as ServerConn,
    test,
};

const UNIT: Duration = Duration::from_millis(200);

/// A client that always reaches the same `ok`-responding server, throttled by `throttle`.
fn ok_client(throttle: Throttle) -> Client {
    Client::new(ServerConnector::new(|conn: ServerConn| async move {
        conn.ok("ok")
    }))
    .with_handler(throttle)
}

/// The client's own paced clock: `Quota::per(1, UNIT)` spaces requests one `UNIT` apart, and
/// `burst` allows that many back-to-back before pacing kicks in.
fn paced(burst: u64) -> Throttle {
    Throttle::new(Quota::per(1, UNIT).allow_burst(burst))
}

#[test(harness)]
async fn spaces_sequential_requests_by_the_interval() -> TestResult {
    let client = ok_client(paced(1));
    let start = Instant::now();
    for _ in 0..3 {
        let _ = client.get("http://example.com/").await?;
    }
    // First request is immediate; each of the next two waits one UNIT.
    let elapsed = start.elapsed();
    assert!(
        elapsed >= 2 * UNIT,
        "three requests took {elapsed:?}, expected >= {:?}",
        2 * UNIT
    );
    assert!(
        elapsed < 5 * UNIT,
        "three requests took {elapsed:?}, unexpectedly slow"
    );
    Ok(())
}

#[test(harness)]
async fn burst_allows_back_to_back_then_paces() -> TestResult {
    let client = ok_client(paced(3));
    let start = Instant::now();
    // Three requests fit the burst and go immediately...
    for _ in 0..3 {
        let _ = client.get("http://example.com/").await?;
    }
    assert!(start.elapsed() < UNIT, "burst of 3 should be near-instant");

    // ...the fourth must wait most of an interval for a token to accrue.
    let before_fourth = Instant::now();
    let _ = client.get("http://example.com/").await?;
    let fourth = before_fourth.elapsed();
    assert!(
        fourth >= UNIT / 2,
        "fourth request should have paced, took only {fourth:?}"
    );
    Ok(())
}

#[test(harness)]
async fn distinct_origins_are_metered_independently() -> TestResult {
    let client = ok_client(paced(1));
    let start = Instant::now();
    // First request to each origin is immediate — separate budgets, no cross-origin pacing.
    let _ = client.get("http://a.example.com/").await?;
    let _ = client.get("http://b.example.com/").await?;
    assert!(
        start.elapsed() < UNIT,
        "distinct origins should not pace each other"
    );
    Ok(())
}

#[test(harness)]
async fn scope_can_group_hosts_into_one_budget() -> TestResult {
    let client = ok_client(paced(1).with_scope(|url: &trillium_client::Url| {
        // Group by registrable domain (the last two labels).
        let host = url.host_str().unwrap_or_default();
        host.rsplit('.').take(2).collect::<Vec<_>>().join(".")
    }));
    let start = Instant::now();
    // Two subdomains share one budget, so the second paces behind the first.
    let _ = client.get("http://a.example.com/").await?;
    let _ = client.get("http://b.example.com/").await?;
    assert!(
        start.elapsed() >= UNIT,
        "grouped subdomains should share a budget"
    );
    Ok(())
}

#[test(harness)]
async fn concurrent_requests_to_one_origin_are_serialized_to_the_pace() -> TestResult {
    let client = ok_client(paced(1));
    let start = Instant::now();
    let (a, b) = zip(
        client.get("http://example.com/").into_future(),
        client.get("http://example.com/").into_future(),
    )
    .await;
    let _ = a?;
    let _ = b?;
    // Even fired concurrently, the two can't both go at once.
    assert!(
        start.elapsed() >= UNIT,
        "concurrent requests should serialize to the pace"
    );
    Ok(())
}

#[test(harness)]
async fn honors_server_retry_after() -> TestResult {
    // The server asks every caller to wait a second before the next request.
    let server =
        |conn: ServerConn| async move { conn.with_response_header("Retry-After", "1").ok("ok") };
    let client = Client::new(ServerConnector::new(server)).with_handler(paced(1));

    // First request goes right out; the response's Retry-After then delays the second.
    let _ = client.get("http://example.com/").await?;
    let start = Instant::now();
    let _ = client.get("http://example.com/").await?;
    assert!(
        start.elapsed() >= Duration::from_millis(900),
        "second request should honor Retry-After, waited only {:?}",
        start.elapsed()
    );
    Ok(())
}
