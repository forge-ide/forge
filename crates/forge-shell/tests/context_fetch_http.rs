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

use std::net::SocketAddr;

use forge_shell::context_fetch::{
    build_client, build_client_with, fetch_url, fetch_url_with, FetchUrlError, PolicyOptions,
    BEGIN_MARKER, END_MARKER, MAX_BODY_BYTES,
};
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
    )
    .expect("test client builds");
    let url = format!("http://docs.example.test:{}/docs", addr.port());
    let allowed = vec!["docs.example.test".to_string()];

    let out = fetch_url_with(&client, &url, &allowed, &options)
        .await
        .expect("fetch ok");
    assert_eq!(out.status, 200);
    assert!(
        out.body.starts_with(BEGIN_MARKER),
        "body must open with marker: {}",
        out.body
    );
    assert!(
        out.body.ends_with(END_MARKER),
        "body must close with marker: {}",
        out.body
    );
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
    )
    .expect("test client builds");
    let url = format!("http://docs.example.test:{}/big", addr.port());
    let allowed = vec!["docs.example.test".to_string()];

    let out = fetch_url_with(&client, &url, &allowed, &options)
        .await
        .expect("fetch ok");
    assert!(out.truncated, "2x cap response must set truncated=true");
    // body = BEGIN_MARKER + "\n" + body_at_cap + ("\n" if needed) + END_MARKER.
    // So the payload portion is bounded by the cap.
    let inner_len = out.body.len() - BEGIN_MARKER.len() - END_MARKER.len() - 2; // 2 newlines
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
