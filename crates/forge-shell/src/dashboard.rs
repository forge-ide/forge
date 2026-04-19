//! Dashboard provider status probe and 10-second cache.
//!
//! Probes the local Ollama daemon via `OllamaProvider::list_models()` and
//! returns a `ProviderStatus` shaped for the Dashboard's ProviderPanel.
//! Results are cached for `ttl` to avoid hammering the daemon on rapid
//! refreshes.
//!
//! The `#[tauri::command] provider_status` wrapper is gated on the `webview`
//! feature (registered in `window_manager::run`); the underlying
//! `probe_status` and `ProviderStatusCache` are feature-independent so unit
//! tests run on hosts without WebKitGTK.

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use forge_providers::ollama::OllamaProvider;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// Shape returned to the Solid frontend. `error_kind` is populated only when
/// `reachable == false`; it carries a terse technical identifier so the UI can
/// preserve the voice rule of surfacing exact error codes verbatim.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderStatus {
    pub reachable: bool,
    pub base_url: String,
    pub models: Vec<String>,
    pub last_checked: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
}

/// Probe the Ollama daemon at `base_url`. Never panics or errors upward —
/// failure paths become `reachable: false` with a populated `error_kind`.
pub async fn probe_status(base_url: &str, timeout: Duration) -> ProviderStatus {
    let provider = OllamaProvider::new(base_url, "");
    let probe = provider.list_models();

    match tokio::time::timeout(timeout, probe).await {
        Ok(Ok(models)) => ProviderStatus {
            reachable: true,
            base_url: base_url.to_string(),
            models,
            last_checked: Utc::now(),
            error_kind: None,
        },
        Ok(Err(err)) => ProviderStatus {
            reachable: false,
            base_url: base_url.to_string(),
            models: Vec::new(),
            last_checked: Utc::now(),
            error_kind: Some(classify_error(&err)),
        },
        Err(_) => ProviderStatus {
            reachable: false,
            base_url: base_url.to_string(),
            models: Vec::new(),
            last_checked: Utc::now(),
            error_kind: Some("timeout".to_string()),
        },
    }
}

/// Classify an error (and its source chain) into a terse, developer-meaningful
/// kind string. Walks `source()` because reqwest's top-level message often
/// hides the real cause (e.g. "error sending request for url …" wraps the
/// underlying "Connection refused" from hyper/tokio).
fn classify_error(err: &(dyn std::error::Error + 'static)) -> String {
    let mut combined = err.to_string();
    let mut source = err.source();
    while let Some(s) = source {
        combined.push_str(" | ");
        combined.push_str(&s.to_string());
        source = s.source();
    }

    let lower = combined.to_lowercase();
    if lower.contains("connection refused") || lower.contains("econnrefused") {
        "connection refused".to_string()
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "timeout".to_string()
    } else if lower.contains("dns") || lower.contains("resolve") {
        "dns".to_string()
    } else if lower.contains("error sending request") || lower.contains("connect") {
        // reqwest wraps connect-layer failures as "error sending request for
        // url (...)" — treat as a connect error when we can't see the cause.
        "connection refused".to_string()
    } else {
        let trimmed = combined.trim();
        if trimmed.len() > 140 {
            format!("{}…", &trimmed[..140])
        } else {
            trimmed.to_string()
        }
    }
}

/// TTL cache that serves the last probe result when called within `ttl`.
pub struct ProviderStatusCache {
    ttl: Duration,
    inner: Mutex<Option<CacheEntry>>,
}

struct CacheEntry {
    stored_at: Instant,
    status: ProviderStatus,
}

impl ProviderStatusCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            inner: Mutex::new(None),
        }
    }

    /// Returns the cached status if still fresh, otherwise probes `base_url`
    /// and stores the result.
    pub async fn get_or_probe(&self, base_url: &str, timeout: Duration) -> ProviderStatus {
        {
            let guard = self.inner.lock().await;
            if let Some(entry) = guard.as_ref() {
                if entry.stored_at.elapsed() < self.ttl && entry.status.base_url == base_url {
                    return entry.status.clone();
                }
            }
        }

        let status = probe_status(base_url, timeout).await;
        let mut guard = self.inner.lock().await;
        *guard = Some(CacheEntry {
            stored_at: Instant::now(),
            status: status.clone(),
        });
        status
    }
}

/// Default Ollama endpoint for the Tauri command — mirrors
/// `forge_providers::ollama::DEFAULT_BASE_URL`.
pub const DEFAULT_OLLAMA_URL: &str = forge_providers::ollama::DEFAULT_BASE_URL;

/// Probe timeout applied to the Ollama `GET /api/tags` call from the
/// Dashboard. Short enough to keep the UI responsive, long enough to
/// accommodate a daemon under load.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Cache TTL required by the DoD — successive calls within this window reuse
/// the last probe result.
pub const CACHE_TTL: Duration = Duration::from_secs(10);

#[cfg(feature = "webview")]
#[tauri::command]
pub async fn provider_status<R: tauri::Runtime>(
    webview: tauri::Webview<R>,
    cache: tauri::State<'_, ProviderStatusCache>,
) -> std::result::Result<ProviderStatus, String> {
    crate::ipc::require_window_label(&webview, "dashboard")?;
    Ok(cache.get_or_probe(DEFAULT_OLLAMA_URL, PROBE_TIMEOUT).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;

    #[derive(Debug)]
    struct StrErr(&'static str, Option<Box<StrErr>>);
    impl fmt::Display for StrErr {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }
    impl std::error::Error for StrErr {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.1.as_deref().map(|e| e as _)
        }
    }

    #[test]
    fn classify_error_maps_connection_refused() {
        let err = StrErr(
            "error sending request",
            Some(Box::new(StrErr("Connection refused", None))),
        );
        assert_eq!(classify_error(&err), "connection refused");
    }

    #[test]
    fn classify_error_maps_timeout() {
        let err = StrErr("operation timed out", None);
        assert_eq!(classify_error(&err), "timeout");
    }

    #[test]
    fn classify_error_falls_back_to_original_message() {
        let err = StrErr("boom", None);
        assert_eq!(classify_error(&err), "boom");
    }
}
