//! F-359: server-side URL context fetcher with SSRF hardening.
//!
//! The webview-side @-context URL resolver previously issued `fetch()`
//! directly. That design activates an SSRF vector the moment the page CSP
//! is widened to permit third-party `connect-src`: a compromised or
//! prompt-injected renderer could redirect a fetch to a private-range IP
//! (cloud metadata, LAN hosts), insert the response body verbatim into the
//! next LLM turn, and chain prompt-injection across trusted hosts.
//!
//! This module owns the Rust-side enforcement. Public surface is limited:
//!
//! - [`enforce_url_policy`] — pure, synchronous policy check. Runs on the
//!   initial URL and again on every redirect hop; fails closed on any of:
//!   non-http(s) scheme, userinfo (`user:pass@host`), non-standard port,
//!   disallowed host (exact match against caller-provided allowlist), or
//!   an IP literal in a private / loopback / link-local / reserved range.
//! - [`fetch_url`] — async fetch that applies the policy, caps the body at
//!   [`MAX_BODY_BYTES`], limits redirect hops, and wraps the returned body
//!   in the [`BEGIN_MARKER`] / [`END_MARKER`] dual-LLM containment markers
//!   before handing it back to the caller.
//!
//! # Threat model coverage
//!
//! | Vector | Defense |
//! |--------|---------|
//! | non-http(s) scheme (`file://`, `javascript:`) | `enforce_url_policy` scheme gate |
//! | credentials in URL | `enforce_url_policy` userinfo reject |
//! | IP literal to AWS IMDS `169.254.169.254` | [`is_blocked_ip`] (link-local) |
//! | loopback `127.0.0.1` / `::1` | [`is_blocked_ip`] |
//! | private-range `10.*`, `172.16.*`, `192.168.*` | [`is_blocked_ip`] |
//! | non-standard port (22, 3306, etc.) | port gate (80 / 443 only) |
//! | redirect to disallowed host / private IP | `reqwest` policy re-validates each hop |
//! | unbounded response body | 32 KiB streaming cap |
//! | slow-loris / hang | request + connect timeouts |

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use reqwest::Url;
use url::Host;

/// Dual-LLM containment marker opening the untrusted fetched body. The
/// system prompt instructs the model to treat anything between these
/// markers as untrusted context, never as instructions.
pub const BEGIN_MARKER: &str = "<<<BEGIN FETCHED URL body>>>";
/// Dual-LLM containment marker closing the untrusted fetched body.
pub const END_MARKER: &str = "<<<END FETCHED URL body>>>";

/// Maximum bytes returned from a single fetch. Mirrors the existing
/// webview-side truncation budget so the switch to Rust enforcement does
/// not change the prompt shape users have grown used to.
pub const MAX_BODY_BYTES: usize = 32 * 1024;

/// Cap on redirect hops. `reqwest::redirect::Policy::custom` can both
/// count hops *and* re-validate each target, so a compromised host that
/// 302's to a private-range IP is rejected mid-chain.
pub const MAX_REDIRECTS: usize = 5;

/// Per-request deadline. Short enough to bound a slow-loris peer, long
/// enough for a real CDN / mirror.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// TCP-connect deadline. Fails fast on dead peers; the request timeout
/// still covers header + body reads.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Result of a successful fetch. `body` is already wrapped in the
/// dual-LLM containment markers; callers splice it into the prompt
/// verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchUrlOk {
    /// Fetched body wrapped in [`BEGIN_MARKER`] / [`END_MARKER`].
    pub body: String,
    /// HTTP status of the final response.
    pub status: u16,
    /// `Content-Type` header if the peer sent one. Purely informational;
    /// policy does not branch on content type.
    pub content_type: Option<String>,
    /// Whether the body was truncated at [`MAX_BODY_BYTES`].
    pub truncated: bool,
}

/// Errors returned by [`enforce_url_policy`] and [`fetch_url`]. Kept a
/// concrete enum so the IPC layer can format user-facing messages and the
/// tests can pattern-match vectors precisely.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchUrlError {
    /// URL failed to parse.
    #[error("invalid URL: {reason}")]
    InvalidUrl { reason: String },
    /// Scheme is not `http` or `https`.
    #[error("scheme not allowed: {scheme} (only http/https)")]
    SchemeNotAllowed { scheme: String },
    /// URL carries a `user:pass@` component.
    #[error("URL must not carry userinfo credentials")]
    UserinfoPresent,
    /// URL has no parseable host.
    #[error("URL has no host")]
    MissingHost,
    /// Host is not on the caller-supplied allowlist.
    #[error("host not on allowed-hosts list: {host}")]
    HostNotAllowed { host: String },
    /// Host is an IP literal in a blocked range (loopback, link-local,
    /// private, multicast, broadcast, reserved). IP literals are always
    /// rejected — allowlisting an IP literal is a configuration smell,
    /// and DNS rebinding against a hostname is a separate concern that
    /// resolve-once-then-pin mitigations would cover.
    #[error("host is a blocked IP range ({reason}): {ip}")]
    BlockedIpRange { ip: String, reason: &'static str },
    /// Port is not on the allowed list. Policy: 80 and 443 only (plus
    /// absent-port, which `reqwest::Url::port_or_known_default` fills
    /// from the scheme).
    #[error("port not allowed: {port} (only 80/443)")]
    PortNotAllowed { port: u16 },
    /// `reqwest` transport failure (timeout, refused connect, redirect
    /// cap hit, etc.). Kept as a plain string so the policy enum stays
    /// `Clone + Eq`.
    #[error("fetch failed: {reason}")]
    TransportFailed { reason: String },
    /// The peer returned a non-2xx status.
    #[error("fetch returned HTTP {status}")]
    HttpStatus { status: u16 },
}

/// Classify an IP literal against the private/loopback/link-local/reserved
/// ranges. Returns `Some(reason)` if the IP is blocked, `None` otherwise.
///
/// Covers both IPv4 and IPv6. IPv4-mapped IPv6 addresses are unmapped
/// first so `::ffff:127.0.0.1` cannot smuggle loopback through an IPv6
/// literal.
pub fn is_blocked_ip(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => {
            // `::ffff:a.b.c.d` — treat as the embedded IPv4 address so a
            // hostile URL like `http://[::ffff:169.254.169.254]/` is
            // rejected as link-local rather than slipping past as
            // "unknown IPv6".
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return classify_v4(mapped);
            }
            classify_v6(v6)
        }
    }
}

fn classify_v4(ip: Ipv4Addr) -> Option<&'static str> {
    if ip.is_loopback() {
        return Some("loopback");
    }
    if ip.is_link_local() {
        return Some("link-local");
    }
    if ip.is_private() {
        return Some("private");
    }
    if ip.is_broadcast() {
        return Some("broadcast");
    }
    if ip.is_multicast() {
        return Some("multicast");
    }
    if ip.is_unspecified() {
        return Some("unspecified");
    }
    // CGNAT (100.64.0.0/10) — not strictly private but routinely used by
    // ISP NATs and reachable only inside the provider network.
    let octets = ip.octets();
    if octets[0] == 100 && (octets[1] & 0b1100_0000) == 64 {
        return Some("cgnat");
    }
    // 0.0.0.0/8, 192.0.0.0/24, 192.0.2.0/24, 198.18.0.0/15, 198.51.100.0/24,
    // 203.0.113.0/24, 240.0.0.0/4 — reserved / documentation / benchmarking.
    if octets[0] == 0 {
        return Some("reserved");
    }
    if octets[0] == 192 && octets[1] == 0 && octets[2] == 0 {
        return Some("reserved");
    }
    if octets[0] == 192 && octets[1] == 0 && octets[2] == 2 {
        return Some("documentation");
    }
    if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
        return Some("benchmarking");
    }
    if octets[0] == 198 && octets[1] == 51 && octets[2] == 100 {
        return Some("documentation");
    }
    if octets[0] == 203 && octets[1] == 0 && octets[2] == 113 {
        return Some("documentation");
    }
    if (octets[0] & 0xF0) == 0xF0 && octets[0] != 0xFF {
        // 240.0.0.0/4 except the broadcast address (already caught).
        return Some("reserved");
    }
    None
}

fn classify_v6(ip: Ipv6Addr) -> Option<&'static str> {
    if ip.is_loopback() {
        return Some("loopback");
    }
    if ip.is_unspecified() {
        return Some("unspecified");
    }
    if ip.is_multicast() {
        return Some("multicast");
    }
    let segments = ip.segments();
    // fe80::/10 — link-local unicast.
    if (segments[0] & 0xFFC0) == 0xFE80 {
        return Some("link-local");
    }
    // fc00::/7 — unique-local (ULA), the IPv6 equivalent of RFC1918.
    if (segments[0] & 0xFE00) == 0xFC00 {
        return Some("unique-local");
    }
    // 2001:db8::/32 — documentation.
    if segments[0] == 0x2001 && segments[1] == 0x0DB8 {
        return Some("documentation");
    }
    // 64:ff9b::/96 — NAT64 well-known prefix. Routing through an NAT64
    // gateway could reach private IPv4 space; reject defensively.
    if segments[0] == 0x0064 && segments[1] == 0xFF9B {
        return Some("nat64-wkp");
    }
    None
}

/// Extra knobs for the URL policy. Exists so integration tests can
/// exercise the hardened transport against a `wiremock` fixture on a
/// random ephemeral port without relaxing the production policy itself.
/// Production call sites use [`Default`] — port 80 and 443 only.
#[derive(Debug, Clone, Default)]
pub struct PolicyOptions {
    /// Extra TCP ports to accept in addition to the default 80/443.
    /// Empty in production. Tests append the wiremock's random port.
    pub extra_ports: Vec<u16>,
}

/// Run the synchronous URL-policy check under default options (80/443
/// only; no extra ports). Thin wrapper over [`enforce_url_policy_with`]
/// kept for call-site ergonomics — production use sites pass the default.
pub fn enforce_url_policy(url: &Url, allowed_hosts: &[String]) -> Result<(), FetchUrlError> {
    enforce_url_policy_with(url, allowed_hosts, &PolicyOptions::default())
}

/// Run the synchronous URL-policy check. Used both on the initial URL
/// (inside [`fetch_url`]) and on every redirect hop (inside the reqwest
/// custom redirect policy). Pure — no network I/O.
///
/// Policy order is deliberate: scheme → userinfo → host present → IP
/// classify → host allowlist → port. A blocked IP literal always fails,
/// even if a user had (misguidedly) allowlisted the IP string, because
/// allowlisting `127.0.0.1` is a configuration smell and leaving the
/// escape hatch open defeats the redirect-re-validation guarantee.
pub fn enforce_url_policy_with(
    url: &Url,
    allowed_hosts: &[String],
    options: &PolicyOptions,
) -> Result<(), FetchUrlError> {
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(FetchUrlError::SchemeNotAllowed {
                scheme: other.to_string(),
            });
        }
    }

    if !url.username().is_empty() || url.password().is_some() {
        return Err(FetchUrlError::UserinfoPresent);
    }

    let host = url.host().ok_or(FetchUrlError::MissingHost)?;

    // `Url::host()` returns a typed `Host<&str>` discriminated into
    // `Domain | Ipv4 | Ipv6`. Using the typed form sidesteps the
    // `host_str()` footgun where IPv6 literals are bracketed (`"[::1]"`)
    // and therefore fail `parse::<IpAddr>()`. Tested by
    // `rejects_loopback_ipv6` / `rejects_ipv6_link_local` — those RED
    // before the switch.
    let host_name = match host {
        Host::Ipv4(v4) => {
            let ip = IpAddr::V4(v4);
            if let Some(reason) = is_blocked_ip(ip) {
                return Err(FetchUrlError::BlockedIpRange {
                    ip: ip.to_string(),
                    reason,
                });
            }
            return Err(FetchUrlError::HostNotAllowed {
                host: ip.to_string(),
            });
        }
        Host::Ipv6(v6) => {
            let ip = IpAddr::V6(v6);
            if let Some(reason) = is_blocked_ip(ip) {
                return Err(FetchUrlError::BlockedIpRange {
                    ip: ip.to_string(),
                    reason,
                });
            }
            return Err(FetchUrlError::HostNotAllowed {
                host: ip.to_string(),
            });
        }
        Host::Domain(name) => name,
    };

    // DNS name — `Url` already lowercases. Hostname match is
    // case-insensitive for defense in depth against future url-crate
    // canonicalization changes.
    let host_lc = host_name.to_ascii_lowercase();
    let allowed = allowed_hosts
        .iter()
        .any(|h| h.eq_ignore_ascii_case(&host_lc));
    if !allowed {
        return Err(FetchUrlError::HostNotAllowed { host: host_lc });
    }

    let port = url
        .port_or_known_default()
        .ok_or(FetchUrlError::PortNotAllowed { port: 0 })?;
    if !is_allowed_port(port, options) {
        return Err(FetchUrlError::PortNotAllowed { port });
    }

    Ok(())
}

/// Allowed port list. Strict — only 80 (http) and 443 (https) in
/// production. `options.extra_ports` is test-only and always empty when
/// called from the default policy path. A host serving its context on
/// an alt port must be reachable via a redirect or a separate entry; the
/// goal is to deny a webview-driven scan of private services on port 22
/// / 3306 / 5432 / etc.
fn is_allowed_port(port: u16, options: &PolicyOptions) -> bool {
    matches!(port, 80 | 443) || options.extra_ports.contains(&port)
}

/// Wrap `body` in the dual-LLM containment markers. Appends a newline
/// before the end marker so the marker always sits on its own line, even
/// if the fetched body doesn't end in `\n`.
pub fn wrap_with_markers(body: &str) -> String {
    let trailing_nl = if body.ends_with('\n') { "" } else { "\n" };
    format!("{BEGIN_MARKER}\n{body}{trailing_nl}{END_MARKER}")
}

/// Truncate a byte slice to `max_bytes`, respecting UTF-8 codepoint
/// boundaries. Returns `(truncated_string, was_truncated)`. Used to cap
/// the returned body at [`MAX_BODY_BYTES`].
pub fn truncate_utf8(bytes: &[u8], max_bytes: usize) -> (String, bool) {
    if bytes.len() <= max_bytes {
        return (String::from_utf8_lossy(bytes).into_owned(), false);
    }
    // Walk back to the last complete codepoint boundary so we don't split
    // a multibyte sequence.
    let mut end = max_bytes;
    while end > 0 && (bytes[end] & 0b1100_0000) == 0b1000_0000 {
        end -= 1;
    }
    (String::from_utf8_lossy(&bytes[..end]).into_owned(), true)
}

/// Build a reqwest client wired with the F-359 hardening defaults:
/// connect + request timeouts, HTTPS-or-HTTP (both are policy-checked),
/// and a custom redirect policy that runs [`enforce_url_policy`] on every
/// hop. Exposed so integration tests can swap the allowlist while pinning
/// identical transport knobs to the production client.
pub fn build_client(allowed_hosts: Vec<String>) -> reqwest::Result<reqwest::Client> {
    build_client_with(allowed_hosts, PolicyOptions::default(), Vec::new())
}

/// Extended builder — test-only seam that also carries a DNS override
/// table and accepts a [`PolicyOptions`] (ports). `dns_resolve` is a
/// list of `(hostname, socketaddr)` pairs piped into
/// [`reqwest::ClientBuilder::resolve`] so `wiremock` fixtures can be
/// reached via a hostname URL while the URL-policy sees the hostname
/// (not the loopback IP), exercising the hardened path exactly as it
/// runs in production. Production callers pass the default options and
/// an empty resolve list.
pub fn build_client_with(
    allowed_hosts: Vec<String>,
    options: PolicyOptions,
    dns_resolve: Vec<(String, std::net::SocketAddr)>,
) -> reqwest::Result<reqwest::Client> {
    let allowed_for_redirect = allowed_hosts.clone();
    let options_for_redirect = options.clone();
    let mut builder = reqwest::Client::builder()
        .user_agent(concat!("forge-shell/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            match enforce_url_policy_with(
                attempt.url(),
                &allowed_for_redirect,
                &options_for_redirect,
            ) {
                Ok(()) => attempt.follow(),
                Err(err) => attempt.error(err.to_string()),
            }
        }));
    for (host, addr) in dns_resolve {
        builder = builder.resolve(&host, addr);
    }
    builder.build()
}

/// Fetch `url` under the F-359 SSRF policy and return its body wrapped in
/// the dual-LLM containment markers. The body is capped at
/// [`MAX_BODY_BYTES`] — excess is dropped silently (but the
/// [`FetchUrlOk::truncated`] flag surfaces the cut to callers).
///
/// Caller supplies a pre-built [`reqwest::Client`] (see [`build_client`])
/// so a single client — with its connection pool — is reused across
/// fetches. The allowlist passed here must match the one used to build
/// the client; the redirect policy re-runs against the client's copy.
pub async fn fetch_url(
    client: &reqwest::Client,
    url: &str,
    allowed_hosts: &[String],
) -> Result<FetchUrlOk, FetchUrlError> {
    fetch_url_with(client, url, allowed_hosts, &PolicyOptions::default()).await
}

/// Test seam: identical to [`fetch_url`] but threads a [`PolicyOptions`]
/// through the initial-URL policy check. The client's redirect policy
/// captures its own copy of options at build time; callers pairing this
/// with [`build_client_with`] must pass the same `PolicyOptions` to both.
pub async fn fetch_url_with(
    client: &reqwest::Client,
    url: &str,
    allowed_hosts: &[String],
    options: &PolicyOptions,
) -> Result<FetchUrlOk, FetchUrlError> {
    let parsed = Url::parse(url).map_err(|e| FetchUrlError::InvalidUrl {
        reason: e.to_string(),
    })?;
    enforce_url_policy_with(&parsed, allowed_hosts, options)?;

    let resp = client
        .get(parsed)
        .send()
        .await
        .map_err(|e| FetchUrlError::TransportFailed {
            reason: e.to_string(),
        })?;

    let status = resp.status();
    let status_u16 = status.as_u16();
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .map(str::to_string);

    if !status.is_success() {
        return Err(FetchUrlError::HttpStatus { status: status_u16 });
    }

    // Stream the body chunk-by-chunk so we can short-circuit on oversize
    // before buffering arbitrary data. `.bytes()` would buffer the whole
    // response first — fine for a 32 KiB cap but the chunked form keeps
    // peak allocation bounded under a hostile peer that sends a large
    // `Content-Length` header with a slow body.
    let mut acc: Vec<u8> = Vec::with_capacity(MAX_BODY_BYTES.min(4096));
    let mut stream = resp;
    let mut truncated = false;
    loop {
        match stream.chunk().await {
            Ok(Some(chunk)) => {
                if acc.len() + chunk.len() > MAX_BODY_BYTES {
                    let room = MAX_BODY_BYTES.saturating_sub(acc.len());
                    acc.extend_from_slice(&chunk[..room]);
                    truncated = true;
                    break;
                }
                acc.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) => {
                return Err(FetchUrlError::TransportFailed {
                    reason: e.to_string(),
                });
            }
        }
    }

    let (body_str, trunc_utf8) = truncate_utf8(&acc, MAX_BODY_BYTES);
    let was_truncated = truncated || trunc_utf8;
    Ok(FetchUrlOk {
        body: wrap_with_markers(&body_str),
        status: status_u16,
        content_type,
        truncated: was_truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn parse(url: &str) -> Url {
        Url::parse(url).expect("test URL parses")
    }

    // ---- SSRF vector: non-http(s) schemes ----

    #[test]
    fn rejects_file_scheme() {
        let err = enforce_url_policy(&parse("file:///etc/passwd"), &hosts(&["etc"])).unwrap_err();
        assert!(
            matches!(err, FetchUrlError::SchemeNotAllowed { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_javascript_scheme() {
        let err =
            enforce_url_policy(&parse("javascript:alert(1)"), &hosts(&["whatever"])).unwrap_err();
        assert!(
            matches!(err, FetchUrlError::SchemeNotAllowed { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_gopher_scheme() {
        let err = enforce_url_policy(
            &parse("gopher://evil.example.com/"),
            &hosts(&["evil.example.com"]),
        )
        .unwrap_err();
        assert!(
            matches!(err, FetchUrlError::SchemeNotAllowed { .. }),
            "{err:?}"
        );
    }

    // ---- SSRF vector: credentials in URL ----

    #[test]
    fn rejects_userinfo_even_for_allowed_host() {
        let err = enforce_url_policy(
            &parse("https://user:pass@example.com/"),
            &hosts(&["example.com"]),
        )
        .unwrap_err();
        assert!(matches!(err, FetchUrlError::UserinfoPresent), "{err:?}");
    }

    #[test]
    fn rejects_userinfo_username_only() {
        let err = enforce_url_policy(
            &parse("https://admin@example.com/"),
            &hosts(&["example.com"]),
        )
        .unwrap_err();
        assert!(matches!(err, FetchUrlError::UserinfoPresent), "{err:?}");
    }

    // ---- SSRF vector: IP literal ranges (AWS IMDS + loopback + LAN) ----

    #[test]
    fn rejects_aws_imds_link_local() {
        // 169.254.169.254 — the EC2/ECS instance metadata service. A
        // prompt-injected webview redirecting an @-fetch here could pull
        // IAM creds and funnel them into the next LLM turn.
        let err = enforce_url_policy(
            &parse("http://169.254.169.254/latest/meta-data/"),
            &hosts(&["169.254.169.254"]),
        )
        .unwrap_err();
        assert!(
            matches!(
                err,
                FetchUrlError::BlockedIpRange {
                    reason: "link-local",
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_loopback_ipv4() {
        let err = enforce_url_policy(
            &parse("http://127.0.0.1:8080/admin"),
            &hosts(&["127.0.0.1"]),
        )
        .unwrap_err();
        assert!(
            matches!(
                err,
                FetchUrlError::BlockedIpRange {
                    reason: "loopback",
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_loopback_ipv6() {
        let err =
            enforce_url_policy(&parse("http://[::1]/"), &hosts(&["[::1]", "::1"])).unwrap_err();
        assert!(
            matches!(
                err,
                FetchUrlError::BlockedIpRange {
                    reason: "loopback",
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_ipv4_mapped_ipv6_loopback() {
        // `::ffff:127.0.0.1` — IPv4-mapped IPv6 form of loopback. Must
        // unmap before classifying or the v4 range checks miss it.
        let err = enforce_url_policy(&parse("http://[::ffff:127.0.0.1]/"), &hosts(&["127.0.0.1"]))
            .unwrap_err();
        assert!(
            matches!(
                err,
                FetchUrlError::BlockedIpRange {
                    reason: "loopback",
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_private_10_range() {
        let err = enforce_url_policy(&parse("http://10.0.0.5/config"), &hosts(&["10.0.0.5"]))
            .unwrap_err();
        assert!(
            matches!(
                err,
                FetchUrlError::BlockedIpRange {
                    reason: "private",
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_private_192_168_range() {
        let err = enforce_url_policy(&parse("http://192.168.1.1/"), &hosts(&["192.168.1.1"]))
            .unwrap_err();
        assert!(
            matches!(
                err,
                FetchUrlError::BlockedIpRange {
                    reason: "private",
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_private_172_16_range() {
        let err =
            enforce_url_policy(&parse("http://172.20.5.5/"), &hosts(&["172.20.5.5"])).unwrap_err();
        assert!(
            matches!(
                err,
                FetchUrlError::BlockedIpRange {
                    reason: "private",
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_ipv6_ula() {
        // fc00::/7 — IPv6 unique-local (RFC1918 equivalent).
        let err =
            enforce_url_policy(&parse("http://[fc00::1]/"), &hosts(&["[fc00::1]"])).unwrap_err();
        assert!(
            matches!(
                err,
                FetchUrlError::BlockedIpRange {
                    reason: "unique-local",
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_ipv6_link_local() {
        let err =
            enforce_url_policy(&parse("http://[fe80::1]/"), &hosts(&["[fe80::1]"])).unwrap_err();
        assert!(
            matches!(
                err,
                FetchUrlError::BlockedIpRange {
                    reason: "link-local",
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_cgnat_range() {
        let err =
            enforce_url_policy(&parse("http://100.64.0.1/"), &hosts(&["100.64.0.1"])).unwrap_err();
        assert!(
            matches!(
                err,
                FetchUrlError::BlockedIpRange {
                    reason: "cgnat",
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_unspecified_zero_addr() {
        let err = enforce_url_policy(&parse("http://0.0.0.0/"), &hosts(&["0.0.0.0"])).unwrap_err();
        assert!(
            matches!(err, FetchUrlError::BlockedIpRange { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_public_ip_literal_outright() {
        // Even a public IP literal (8.8.8.8) must be rejected — users
        // allowlist by name, and name-based policy is what the DNS
        // indirection lets us pin. Allowing `https://8.8.8.8/` would
        // skip every pinning concept this module enforces.
        let err = enforce_url_policy(&parse("https://8.8.8.8/"), &hosts(&["8.8.8.8"])).unwrap_err();
        assert!(
            matches!(err, FetchUrlError::HostNotAllowed { .. }),
            "{err:?}"
        );
    }

    // ---- SSRF vector: port scanning ----

    #[test]
    fn rejects_non_standard_port() {
        // A webview shouldn't be able to probe `https://host:22/` on an
        // allowlisted host — port 22 is SSH, and the one-sided TLS
        // handshake alone is a fingerprint.
        let err =
            enforce_url_policy(&parse("https://docs.rs:22/"), &hosts(&["docs.rs"])).unwrap_err();
        assert!(
            matches!(err, FetchUrlError::PortNotAllowed { port: 22 }),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_mysql_port() {
        let err = enforce_url_policy(&parse("http://example.com:3306/"), &hosts(&["example.com"]))
            .unwrap_err();
        assert!(
            matches!(err, FetchUrlError::PortNotAllowed { port: 3306 }),
            "{err:?}"
        );
    }

    // ---- Allowlist matching ----

    #[test]
    fn rejects_disallowed_host_exact_match() {
        // Substring / suffix matching is a footgun (`example.com` should
        // NOT match `evil-example.com` or `example.com.attacker.com`).
        // Exact hostname match only.
        let err = enforce_url_policy(
            &parse("https://evil-example.com/"),
            &hosts(&["example.com"]),
        )
        .unwrap_err();
        assert!(
            matches!(err, FetchUrlError::HostNotAllowed { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn rejects_suffix_bypass_attempt() {
        let err = enforce_url_policy(
            &parse("https://example.com.attacker.com/"),
            &hosts(&["example.com"]),
        )
        .unwrap_err();
        assert!(
            matches!(err, FetchUrlError::HostNotAllowed { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn allows_matching_host_and_port_443() {
        let ok = enforce_url_policy(&parse("https://docs.rs/tokio"), &hosts(&["docs.rs"]));
        assert!(ok.is_ok(), "{ok:?}");
    }

    #[test]
    fn allows_matching_host_and_port_80() {
        let ok = enforce_url_policy(&parse("http://example.com/"), &hosts(&["example.com"]));
        assert!(ok.is_ok(), "{ok:?}");
    }

    #[test]
    fn hostname_match_is_case_insensitive() {
        let ok = enforce_url_policy(&parse("https://DOCS.RS/tokio"), &hosts(&["docs.rs"]));
        assert!(ok.is_ok(), "{ok:?}");
        let ok2 = enforce_url_policy(&parse("https://docs.rs/tokio"), &hosts(&["DOCS.RS"]));
        assert!(ok2.is_ok(), "{ok2:?}");
    }

    // ---- Marker wrapping ----

    #[test]
    fn wrap_with_markers_brackets_body() {
        let wrapped = wrap_with_markers("the page body");
        assert!(wrapped.starts_with(BEGIN_MARKER));
        assert!(wrapped.contains("the page body"));
        assert!(wrapped.ends_with(END_MARKER));
    }

    #[test]
    fn wrap_preserves_trailing_newline() {
        let wrapped = wrap_with_markers("body\n");
        assert!(
            !wrapped.contains("\n\n"),
            "should not double-newline: {wrapped}"
        );
        assert!(wrapped.ends_with(END_MARKER));
    }

    // ---- UTF-8 truncation ----

    #[test]
    fn truncate_utf8_noop_when_small() {
        let (s, t) = truncate_utf8(b"hello", 100);
        assert_eq!(s, "hello");
        assert!(!t);
    }

    #[test]
    fn truncate_utf8_does_not_split_multibyte() {
        // "héllo" encodes `é` as 2 bytes (0xC3 0xA9). Truncating at 2
        // would land inside the sequence. The helper must walk back.
        let input = "hé".as_bytes(); // h=1, é=2 -> 3 bytes
        let (s, t) = truncate_utf8(input, 2);
        assert!(t);
        assert_eq!(s, "h");
    }

    // ---- Schemas that reqwest handles but we must block ----

    #[test]
    fn rejects_missing_host() {
        // `http:///path` parses but has no host. Must fail closed.
        if let Ok(url) = Url::parse("http:///just-a-path") {
            let err = enforce_url_policy(&url, &hosts(&["x"])).unwrap_err();
            assert!(
                matches!(
                    err,
                    FetchUrlError::MissingHost | FetchUrlError::HostNotAllowed { .. }
                ),
                "{err:?}"
            );
        }
    }
}
