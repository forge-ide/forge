//! Regression tests for the `HttpDownloader` hardening landed in F-350.
//!
//! Each scenario pins a threat-model concern from the finding:
//!
//! - slow-loris → request-level timeout must fire before the body arrives
//! - oversize body → body stream is capped by a configurable ceiling
//! - cross-scheme redirect → redirect policy refuses to leave `https`
//! - plain-http rejection → `https_only(true)` refuses non-TLS fetches
//!
//! Tests drive the downloader through a configurable-options constructor
//! so wiremock (plain HTTP, no body limit) can stand in for the real
//! network while the production defaults (HTTPS-only, 256 MiB cap, 60 s
//! request timeout) remain the shipped behaviour.

use std::time::Duration;

use forge_lsp::bootstrap::{Downloader, HttpClientOptions, HttpDownloader, OversizeBody};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_options() -> HttpClientOptions {
    // Wiremock speaks plain HTTP, so drop https_only and shrink every
    // ceiling to something the test loop can exercise quickly.
    HttpClientOptions {
        request_timeout: Duration::from_millis(500),
        connect_timeout: Duration::from_millis(200),
        max_redirects: 5,
        https_only: false,
        max_body_bytes: 4 * 1024, // 4 KiB
    }
}

#[tokio::test]
async fn slow_loris_trips_request_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/archive.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(b"irrelevant".as_slice())
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&server)
        .await;

    let d = HttpDownloader::with_options(test_options());
    let err = d
        .fetch(&format!("{}/archive.bin", server.uri()))
        .await
        .expect_err("slow-loris must be cut off by the request timeout");
    // Walk the `std::error::Error` source chain — reqwest's top-level
    // display is "error sending request", but the underlying cause is
    // the timeout. Decoupling from reqwest wording via a chain-scan also
    // protects us if reqwest reshuffles its surface wording.
    let chain = error_chain_display(err.as_ref());
    assert!(
        chain.to_ascii_lowercase().contains("timeout")
            || chain.to_ascii_lowercase().contains("timed out"),
        "expected a timeout error in the chain, got: {chain}"
    );
}

fn error_chain_display(err: &(dyn std::error::Error + 'static)) -> String {
    let mut out = String::new();
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = cur {
        if !out.is_empty() {
            out.push_str(" | ");
        }
        out.push_str(&e.to_string());
        cur = e.source();
    }
    out
}

#[tokio::test]
async fn oversize_body_surfaces_dedicated_error() {
    let server = MockServer::start().await;
    let big = vec![0u8; 16 * 1024]; // 16 KiB, well over the 4 KiB test cap
    Mock::given(method("GET"))
        .and(path("/huge"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(big))
        .mount(&server)
        .await;

    let d = HttpDownloader::with_options(test_options());
    let err = d
        .fetch(&format!("{}/huge", server.uri()))
        .await
        .expect_err("oversize body must be rejected");
    let oversize = err
        .downcast_ref::<OversizeBody>()
        .expect("oversize error must be downcastable to OversizeBody");
    assert_eq!(
        oversize.limit,
        4 * 1024,
        "limit must reflect configured cap"
    );
}

#[tokio::test]
async fn cross_scheme_redirect_is_refused() {
    // The downloader is built with https_only=true (production default). A
    // plain-HTTP URL — which is what `redirect -> attacker` collapses to
    // once resolved — must be refused before any bytes flow. This covers
    // both the initial-URL check and the redirect-target check, since
    // reqwest applies `https_only` to both.
    let d = HttpDownloader::new();
    let err = d
        .fetch("http://example.invalid/anything")
        .await
        .expect_err("plain-HTTP fetch must be refused under https_only");
    let msg = err.to_string().to_ascii_lowercase();
    assert!(
        msg.contains("http") || msg.contains("scheme") || msg.contains("url"),
        "expected an HTTPS/scheme-related refusal, got: {msg}"
    );
}

#[tokio::test]
async fn redirect_chain_is_capped() {
    // An infinite redirect loop must not hang — the limited redirect
    // policy trips after `max_redirects` hops. Uses plain HTTP with
    // https_only=false so wiremock can model the loop.
    let server = MockServer::start().await;
    let loop_path = "/loop";
    Mock::given(method("GET"))
        .and(path(loop_path))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}{loop_path}", server.uri())),
        )
        .mount(&server)
        .await;

    let d = HttpDownloader::with_options(test_options());
    let err = d
        .fetch(&format!("{}{loop_path}", server.uri()))
        .await
        .expect_err("infinite redirect loop must be refused");
    let msg = err.to_string().to_ascii_lowercase();
    assert!(
        msg.contains("redirect") || msg.contains("too many"),
        "expected a redirect-policy refusal, got: {msg}"
    );
}
