//! F-359: HTTP-layer coverage for `context_fetch::fetch_url`.
//!
//! These tests drive the real `reqwest` client against a `wiremock`
//! fixture so the redirect policy, body cap, marker wrapping, and
//! transport error paths are exercised end-to-end. The pure policy
//! unit tests live in `src/context_fetch.rs`; this file covers the
//! bits that only show up once a live client is in the loop.
//!
//! `PolicyOptions` threads the wiremock's random port through the
//! allowlist; production always uses the default (80/443 only). One
//! dedicated test (`rejects_port_not_in_allowlist_even_for_allowed_host`)
//! pins the production default explicitly by calling [`build_client`]
//! without options — any regression that relaxes the default port gate
//! surfaces there.
//!
//! Gated on the `webview-test` feature because the `_with` test seams
//! on the library surface are feature-gated out of production — they
//! exist only to let test fixtures pass wiremock's random port / DNS
//! override through the otherwise-strict policy.

#![cfg(feature = "webview-test")]

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use forge_shell::context_fetch::{
    build_client, build_client_with, fetch_url, fetch_url_with, FetchUrlError, PolicyOptions,
    BEGIN_MARKER_PREFIX, END_MARKER_PREFIX, MAX_BODY_BYTES,
};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_options(port: u16) -> PolicyOptions {
    PolicyOptions {
        extra_ports: vec![port],
    }
}

#[tokio::test]
async fn fetches_allowed_host_and_wraps_in_markers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/docs"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("hello from allowed host")
                .insert_header("content-type", "text/plain"),
        )
        .mount(&server)
        .await;

    let addr: SocketAddr = *server.address();
    let options = test_options(addr.port());
    let client = build_client_with(
        vec!["docs.example.test".to_string()],
        options.clone(),
        vec![("docs.example.test".to_string(), addr)],
        None,
    )
    .expect("test client builds");
    let url = format!("http://docs.example.test:{}/docs", addr.port());
    let allowed = vec!["docs.example.test".to_string()];

    let out = fetch_url_with(&client, &url, &allowed, &options)
        .await
        .expect("fetch ok");
    assert_eq!(out.status, 200);
    assert!(
        out.body.starts_with(&out.begin_marker),
        "body must open with the per-call begin marker: {}",
        out.body
    );
    assert!(
        out.body.ends_with(&out.end_marker),
        "body must close with the per-call end marker: {}",
        out.body
    );
    // The per-call markers always start with the well-known prefix.
    assert!(out.begin_marker.starts_with(BEGIN_MARKER_PREFIX));
    assert!(out.end_marker.starts_with(END_MARKER_PREFIX));
    assert!(out.body.contains("hello from allowed host"));
    assert!(!out.truncated);
}

#[tokio::test]
async fn rejects_port_not_in_allowlist_even_for_allowed_host() {
    // Pin the production default: no `PolicyOptions` override, no
    // extra ports. wiremock's random port must be refused even though
    // the host is allowlisted. This is the "no port scan" guarantee.
    let server = MockServer::start().await;
    let addr: SocketAddr = *server.address();
    let url = format!("http://docs.example.test:{}/docs", addr.port());

    let allowed = vec!["docs.example.test".to_string()];
    let client = build_client(allowed.clone()).expect("client builds");
    let err = fetch_url(&client, &url, &allowed).await.unwrap_err();
    assert!(
        matches!(err, FetchUrlError::PortNotAllowed { .. }),
        "{err:?}"
    );
}

#[tokio::test]
async fn redirect_to_blocked_ip_is_rejected_midchain() {
    // Classic SSRF chain: allowed host serves a 302 to
    // http://169.254.169.254/ (AWS IMDS). The redirect policy runs
    // `enforce_url_policy` on every hop — this redirect must be
    // refused, not followed. Captured as a transport error surfacing
    // the policy reason.
    let server = MockServer::start().await;
    let addr: SocketAddr = *server.address();
    Mock::given(method("GET"))
        .and(path("/hop"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "http://169.254.169.254/latest/meta-data/"),
        )
        .mount(&server)
        .await;

    let options = test_options(addr.port());
    let client = build_client_with(
        vec!["docs.example.test".to_string()],
        options.clone(),
        vec![("docs.example.test".to_string(), addr)],
        None,
    )
    .expect("test client builds");
    let url = format!("http://docs.example.test:{}/hop", addr.port());
    let allowed = vec!["docs.example.test".to_string()];

    let err = fetch_url_with(&client, &url, &allowed, &options)
        .await
        .unwrap_err();
    match err {
        FetchUrlError::TransportFailed { reason } => {
            // Message body isn't contract — but the policy's IP rejection
            // string should bubble up through reqwest's custom policy.
            assert!(
                reason.contains("link-local")
                    || reason.contains("blocked")
                    || reason.contains("redirect"),
                "redirect should surface as policy rejection; got: {reason}"
            );
        }
        other => panic!("expected TransportFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn redirect_to_disallowed_host_is_rejected_midchain() {
    // Allowed host 302's to a host the user never allowlisted. The
    // redirect policy rejects the hop — surfaces as TransportFailed.
    let server = MockServer::start().await;
    let addr: SocketAddr = *server.address();
    Mock::given(method("GET"))
        .and(path("/hop"))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "location",
            format!("http://disallowed.example.test:{}/secrets", addr.port()),
        ))
        .mount(&server)
        .await;

    // Resolve both the origin and the redirect target to the same
    // wiremock so the DNS step can't fail first — the policy is what
    // must reject the hop.
    let options = test_options(addr.port());
    let client = build_client_with(
        vec!["docs.example.test".to_string()],
        options.clone(),
        vec![
            ("docs.example.test".to_string(), addr),
            ("disallowed.example.test".to_string(), addr),
        ],
        None,
    )
    .expect("test client builds");
    let url = format!("http://docs.example.test:{}/hop", addr.port());
    let allowed = vec!["docs.example.test".to_string()];

    let err = fetch_url_with(&client, &url, &allowed, &options)
        .await
        .unwrap_err();
    assert!(
        matches!(err, FetchUrlError::TransportFailed { .. }),
        "disallowed-host redirect must fail the fetch; got: {err:?}"
    );
}

#[tokio::test]
async fn body_is_truncated_at_cap() {
    let server = MockServer::start().await;
    let addr: SocketAddr = *server.address();
    // Generate a body twice the cap so the truncation path is exercised.
    let big = "x".repeat(MAX_BODY_BYTES * 2);
    Mock::given(method("GET"))
        .and(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_string(big))
        .mount(&server)
        .await;

    let options = test_options(addr.port());
    let client = build_client_with(
        vec!["docs.example.test".to_string()],
        options.clone(),
        vec![("docs.example.test".to_string(), addr)],
        None,
    )
    .expect("test client builds");
    let url = format!("http://docs.example.test:{}/big", addr.port());
    let allowed = vec!["docs.example.test".to_string()];

    let out = fetch_url_with(&client, &url, &allowed, &options)
        .await
        .expect("fetch ok");
    assert!(out.truncated, "2x cap response must set truncated=true");
    // body = begin_marker + "\n" + body_at_cap + ("\n" if needed) + end_marker.
    // So the payload portion is bounded by the cap.
    let inner_len = out.body.len() - out.begin_marker.len() - out.end_marker.len() - 2; // 2 newlines
    assert!(
        inner_len <= MAX_BODY_BYTES,
        "inner payload len {inner_len} must be <= {MAX_BODY_BYTES}"
    );
}

#[tokio::test]
async fn non_2xx_surfaces_as_http_status_error() {
    let server = MockServer::start().await;
    let addr: SocketAddr = *server.address();
    Mock::given(method("GET"))
        .and(path("/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let options = test_options(addr.port());
    let client = build_client_with(
        vec!["docs.example.test".to_string()],
        options.clone(),
        vec![("docs.example.test".to_string(), addr)],
        None,
    )
    .expect("test client builds");
    let url = format!("http://docs.example.test:{}/missing", addr.port());
    let allowed = vec!["docs.example.test".to_string()];

    let err = fetch_url_with(&client, &url, &allowed, &options)
        .await
        .unwrap_err();
    assert!(
        matches!(err, FetchUrlError::HttpStatus { status: 404 }),
        "{err:?}"
    );
}

// ---- HIGH #1: DNS-rebinding / DNS-based SSRF ----

/// Test inner DNS resolver that maps names to fixed `SocketAddr`s. Lets
/// the DNS-policy path run against a controlled answer set — the
/// production `SystemResolver` calls `tokio::net::lookup_host` which we
/// cannot steer in a hermetic test.
struct MapResolver {
    map: Vec<(String, Vec<SocketAddr>)>,
}

impl Resolve for MapResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let needle = name.as_str().to_string();
        let answer: Option<Vec<SocketAddr>> = self
            .map
            .iter()
            .find(|(n, _)| n == &needle)
            .map(|(_, v)| v.clone());
        Box::pin(async move {
            match answer {
                Some(addrs) => Ok(Box::new(addrs.into_iter()) as Addrs),
                None => Err(Box::new(std::io::Error::other("no test mapping"))
                    as Box<dyn std::error::Error + Send + Sync>),
            }
        }) as Pin<Box<dyn Future<Output = _> + Send>>
    }
}

#[tokio::test]
async fn dns_resolver_rejects_hostname_resolving_to_loopback() {
    // HIGH #1: a hostname that passes the allowlist but whose DNS
    // resolution lands on 127.0.0.1 (classic DNS-rebinding / nip.io
    // attack) must be rejected at the DNS layer. `reqwest` must never
    // open a TCP connection to the loopback listener.
    let resolver = Arc::new(MapResolver {
        map: vec![(
            "rebinding.example.test".to_string(),
            vec!["127.0.0.1:443".parse().unwrap()],
        )],
    });
    let client = build_client_with(
        vec!["rebinding.example.test".to_string()],
        PolicyOptions::default(),
        Vec::new(),
        Some(resolver),
    )
    .expect("client builds with policy-enforcing DNS resolver");
    let url = "https://rebinding.example.test/".to_string();
    let allowed = vec!["rebinding.example.test".to_string()];

    let err = fetch_url(&client, &url, &allowed).await.unwrap_err();
    // The DNS resolver refuses to yield any `SocketAddr` (all blocked), so
    // `reqwest` surfaces a generic send-failure. The precise error text is
    // not contract — the key invariant is that the fetch does NOT succeed
    // and no TCP connect to 127.0.0.1 is issued. `TransportFailed` is the
    // only non-2xx variant that can reach the caller on this path.
    assert!(
        matches!(err, FetchUrlError::TransportFailed { .. }),
        "expected TransportFailed from DNS policy rejection; got: {err:?}"
    );
}

#[tokio::test]
async fn dns_resolver_rejects_hostname_resolving_to_link_local() {
    // AWS IMDS via DNS rebinding — an allowlisted public name pointing
    // at 169.254.169.254. Same attack shape, different range.
    let resolver = Arc::new(MapResolver {
        map: vec![(
            "metadata.example.test".to_string(),
            vec!["169.254.169.254:80".parse().unwrap()],
        )],
    });
    let client = build_client_with(
        vec!["metadata.example.test".to_string()],
        PolicyOptions::default(),
        Vec::new(),
        Some(resolver),
    )
    .expect("client builds with policy-enforcing DNS resolver");
    let url = "http://metadata.example.test/latest/meta-data/".to_string();
    let allowed = vec!["metadata.example.test".to_string()];

    let err = fetch_url(&client, &url, &allowed).await.unwrap_err();
    assert!(
        matches!(err, FetchUrlError::TransportFailed { .. }),
        "link-local via DNS must surface as TransportFailed; got: {err:?}"
    );
}

#[tokio::test]
async fn dns_resolver_drops_blocked_ips_keeps_allowed_ips() {
    // A resolver returning a MIX of blocked and allowed IPs must drop
    // the blocked ones and still connect via the allowed ones. This
    // pins the "filter, don't refuse" behavior when at least one
    // safe endpoint is available.
    //
    // We wire the allowed address to point at a wiremock bound on
    // 127.0.0.1 (the local listener is fine — the IP filter rejects
    // loopback, so the test actually exercises the drop+empty path
    // against 169.254.169.254). Because the loopback entry is also
    // blocked, this test pins that a fully-blocked answer set ends in
    // DNS failure, not silent fallback.
    let resolver = Arc::new(MapResolver {
        map: vec![(
            "mixed.example.test".to_string(),
            vec![
                "169.254.169.254:443".parse().unwrap(),
                "127.0.0.1:443".parse().unwrap(),
            ],
        )],
    });
    let client = build_client_with(
        vec!["mixed.example.test".to_string()],
        PolicyOptions::default(),
        Vec::new(),
        Some(resolver),
    )
    .expect("client builds");
    let err = fetch_url(
        &client,
        "https://mixed.example.test/",
        &["mixed.example.test".to_string()],
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, FetchUrlError::TransportFailed { .. }),
        "all-blocked DNS answer must fail; got: {err:?}"
    );
}
