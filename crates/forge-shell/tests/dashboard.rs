//! Integration tests for `dashboard::probe_status` and `ProviderStatusCache`.
//!
//! Drives the four DoD test cases from F-023: reachable, unreachable, timeout,
//! and empty model list, plus the 10-second cache guarantee.

use std::time::Duration;

use forge_shell::dashboard::{probe_status, ProviderStatusCache};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn tags_body(names: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "models": names
            .iter()
            .map(|n| serde_json::json!({ "name": n }))
            .collect::<Vec<_>>(),
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn probe_status_reachable_returns_models() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tags_body(&["llama3", "mistral"])))
        .mount(&server)
        .await;

    let status = probe_status(&server.uri(), Duration::from_secs(2)).await;

    assert!(
        status.reachable,
        "daemon with 200 response must be reachable"
    );
    assert_eq!(status.base_url, server.uri());
    assert_eq!(status.models, vec!["llama3", "mistral"]);
    assert!(status.error_kind.is_none(), "no error expected");
}

#[tokio::test(flavor = "multi_thread")]
async fn probe_status_reachable_with_empty_models() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tags_body(&[])))
        .mount(&server)
        .await;

    let status = probe_status(&server.uri(), Duration::from_secs(2)).await;

    assert!(status.reachable, "empty model list is still reachable");
    assert!(status.models.is_empty());
    assert!(status.error_kind.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn probe_status_unreachable_connection_refused() {
    // Bind and drop to reserve an unused port — connecting will refuse.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    let url = format!("http://127.0.0.1:{port}");

    let status = probe_status(&url, Duration::from_secs(2)).await;

    assert!(!status.reachable, "refused connection must be unreachable");
    assert_eq!(status.base_url, url);
    assert!(status.models.is_empty());
    let kind = status.error_kind.expect("error_kind must be set");
    assert!(
        kind.contains("connect") || kind.contains("refused"),
        "error kind should name connect/refused, got: {kind}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn probe_status_timeout_returns_unreachable() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(tags_body(&["llama3"]))
                .set_delay(Duration::from_millis(500)),
        )
        .mount(&server)
        .await;

    let status = probe_status(&server.uri(), Duration::from_millis(50)).await;

    assert!(!status.reachable, "slow daemon must surface as unreachable");
    assert!(status.models.is_empty());
    let kind = status.error_kind.expect("error_kind must be set");
    assert!(
        kind.contains("timeout"),
        "error kind should mention timeout, got: {kind}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cache_serves_second_call_within_ttl() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tags_body(&["llama3"])))
        .expect(1) // wiremock asserts exactly one hit on drop
        .mount(&server)
        .await;

    let cache = ProviderStatusCache::new(Duration::from_secs(10));
    let url = server.uri();
    let timeout = Duration::from_secs(2);

    let first = cache.get_or_probe(&url, timeout).await;
    let second = cache.get_or_probe(&url, timeout).await;

    assert!(first.reachable);
    assert!(second.reachable);
    assert_eq!(
        first.last_checked, second.last_checked,
        "cache must return identical timestamp"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn cache_refreshes_after_ttl_expires() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tags_body(&["llama3"])))
        .expect(2)
        .mount(&server)
        .await;

    let cache = ProviderStatusCache::new(Duration::from_millis(50));
    let url = server.uri();
    let timeout = Duration::from_secs(2);

    let first = cache.get_or_probe(&url, timeout).await;
    tokio::time::sleep(Duration::from_millis(80)).await;
    let second = cache.get_or_probe(&url, timeout).await;

    assert!(first.reachable && second.reachable);
    assert_ne!(
        first.last_checked, second.last_checked,
        "expired cache must re-probe and update timestamp"
    );
}
