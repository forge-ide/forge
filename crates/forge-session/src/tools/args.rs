//! Shared argument-extraction helpers for `Tool` implementations.
//!
//! Each tool's `invoke` and `approval_preview` repeats the same
//! `args.get(key).and_then(|v| v.as_str())…` pattern. Concentrating those
//! patterns here keeps validation semantics consistent across tools and gives
//! future tools (and refactors that change the error shape) a single edit
//! point.
//!
//! Three helpers cover every existing call site:
//! - [`get_required_str`] — required string, returns the unified
//!   `MissingRequiredArg` error on miss / wrong-type. Empty string is
//!   intentionally accepted; see the F-074 rationale on the function.
//! - [`get_optional_str`] — optional string, returns `None` on miss /
//!   wrong-type. Used by `approval_preview` (which tolerates missing args
//!   so the user sees the literal request) and by `shell.exec`'s `cwd`.
//! - [`get_optional_u64`] — optional unsigned integer, returns `None` on
//!   miss / wrong-type. Used by `shell.exec`'s `timeout_ms`.
//!
//! All helpers are pure value extractors: they never coerce, never default,
//! and never log. Callers that need a default chain `.unwrap_or(default)`
//! at the use site so the default is visible in context.

use super::ToolError;

/// Extract a required string argument from a tool-call's JSON args object.
///
/// Returns `Err(ToolError::MissingRequiredArg { tool, arg: key })` when the
/// key is absent or the value is not a JSON string. Empty strings are
/// **accepted** — `fs.write` with `{"content": ""}` is a legitimate
/// "truncate file to zero bytes" operation, and rejecting it here would
/// regress that behaviour with a misleading "missing parameter" error
/// (the parameter is supplied; it is just empty). Tools that need to
/// additionally reject empty (e.g. `shell.exec` on `command`) layer that
/// check on top of this helper.
///
/// The `Display` shape of the returned error is contractual:
/// `tool.{tool}: missing required parameter '{arg}'`. IPC-level regression
/// tests assert that exact string — see `tools::tests` and the per-tool
/// tests added in F-074. Do not change the format without updating those
/// tests in lockstep.
pub fn get_required_str<'a>(
    args: &'a serde_json::Value,
    tool: &str,
    key: &str,
) -> Result<&'a str, ToolError> {
    match args.get(key).and_then(|v| v.as_str()) {
        Some(s) => Ok(s),
        None => Err(ToolError::MissingRequiredArg {
            tool: tool.to_string(),
            arg: key.to_string(),
        }),
    }
}

/// Extract an optional string argument from a tool-call's JSON args object.
///
/// Returns `Some(s)` only when `key` is present and the value is a JSON
/// string. Returns `None` for both "key absent" and "wrong type" — these
/// are equivalent for optional args (the caller's `.unwrap_or(default)`
/// or `match` arm handles both uniformly).
///
/// Used by `approval_preview` for required parameters: the preview
/// reflects the literal request so the user can see exactly what the
/// model asked for before consenting. Required-argument validation runs
/// in `invoke`, not in the preview — there is no point rejecting a
/// malformed call until the user has had a chance to refuse it.
pub fn get_optional_str<'a>(args: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

/// Extract an optional `u64` argument from a tool-call's JSON args object.
///
/// Returns `Some(n)` only when `key` is present and the value is a JSON
/// number that fits in `u64` (per `serde_json::Value::as_u64` — i.e.
/// non-negative, integral, within range). Returns `None` for missing
/// keys, wrong types (string, bool, array, object), negative numbers,
/// non-integral floats, and out-of-range integers.
///
/// Used by `shell.exec` for `timeout_ms`. Callers that need a default
/// chain `.unwrap_or(default)` at the use site so the default value
/// stays visible next to the call.
pub fn get_optional_u64(args: &serde_json::Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|v| v.as_u64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- get_required_str (moved from F-074) ----

    #[test]
    fn get_required_str_returns_string_when_present() {
        let v = json!({ "path": "/tmp/x" });
        assert_eq!(get_required_str(&v, "fs.read", "path").unwrap(), "/tmp/x");
    }

    #[test]
    fn get_required_str_accepts_empty_string() {
        // F-074: empty is intentionally allowed so `fs.write` can truncate
        // a file via `{"content": ""}`. Tools that need stricter checks
        // (e.g. `shell.exec` rejecting `""` for `command`) layer the
        // empty-guard on top of this helper.
        let v = json!({ "content": "" });
        assert_eq!(get_required_str(&v, "fs.write", "content").unwrap(), "");
    }

    #[test]
    fn get_required_str_rejects_missing_key_with_unified_shape() {
        let v = json!({});
        let err = get_required_str(&v, "fs.read", "path").unwrap_err();
        assert_eq!(
            err,
            ToolError::MissingRequiredArg {
                tool: "fs.read".to_string(),
                arg: "path".to_string()
            }
        );
        assert_eq!(
            err.to_string(),
            "tool.fs.read: missing required parameter 'path'"
        );
    }

    #[test]
    fn get_required_str_rejects_non_string_value() {
        let v = json!({ "path": 42 });
        let err = get_required_str(&v, "fs.read", "path").unwrap_err();
        assert_eq!(
            err,
            ToolError::MissingRequiredArg {
                tool: "fs.read".to_string(),
                arg: "path".to_string()
            }
        );
    }

    // ---- get_optional_str ----

    #[test]
    fn get_optional_str_returns_some_when_present() {
        let v = json!({ "path": "/tmp/x" });
        assert_eq!(get_optional_str(&v, "path"), Some("/tmp/x"));
    }

    #[test]
    fn get_optional_str_returns_some_for_empty_string() {
        // Symmetric with `get_required_str`: empty is a valid string
        // value, distinct from "absent". Callers that want to treat
        // empty as absent must do so explicitly.
        let v = json!({ "content": "" });
        assert_eq!(get_optional_str(&v, "content"), Some(""));
    }

    #[test]
    fn get_optional_str_returns_none_when_missing() {
        let v = json!({});
        assert_eq!(get_optional_str(&v, "path"), None);
    }

    #[test]
    fn get_optional_str_returns_none_for_non_string_value() {
        // Number, bool, array, object, null — all treated as absent so
        // the caller's `.unwrap_or(default)` fires uniformly.
        for (label, val) in [
            ("number", json!({ "k": 42 })),
            ("bool", json!({ "k": true })),
            ("array", json!({ "k": ["a"] })),
            ("object", json!({ "k": { "x": 1 } })),
            ("null", json!({ "k": null })),
        ] {
            assert_eq!(
                get_optional_str(&val, "k"),
                None,
                "expected None for {label}"
            );
        }
    }

    // ---- get_optional_u64 ----

    #[test]
    fn get_optional_u64_returns_some_when_present() {
        let v = json!({ "timeout_ms": 5000 });
        assert_eq!(get_optional_u64(&v, "timeout_ms"), Some(5000));
    }

    #[test]
    fn get_optional_u64_returns_some_for_zero() {
        // Zero is a valid u64. Callers that treat 0 as "unset" must do
        // so explicitly (none currently do — `shell.exec` clamps via
        // `.min(MAX_TIMEOUT_MS)` and 0 yields an immediate timeout,
        // which is a legitimate caller request).
        let v = json!({ "timeout_ms": 0 });
        assert_eq!(get_optional_u64(&v, "timeout_ms"), Some(0));
    }

    #[test]
    fn get_optional_u64_returns_some_for_u64_max() {
        let v = json!({ "timeout_ms": u64::MAX });
        assert_eq!(get_optional_u64(&v, "timeout_ms"), Some(u64::MAX));
    }

    #[test]
    fn get_optional_u64_returns_none_when_missing() {
        let v = json!({});
        assert_eq!(get_optional_u64(&v, "timeout_ms"), None);
    }

    #[test]
    fn get_optional_u64_returns_none_for_non_numeric() {
        for (label, val) in [
            ("string", json!({ "k": "5000" })),
            ("bool", json!({ "k": true })),
            ("array", json!({ "k": [5000] })),
            ("object", json!({ "k": { "ms": 5000 } })),
            ("null", json!({ "k": null })),
        ] {
            assert_eq!(
                get_optional_u64(&val, "k"),
                None,
                "expected None for {label}"
            );
        }
    }

    #[test]
    fn get_optional_u64_returns_none_for_negative() {
        // Negative numbers fail `as_u64` even though they are JSON
        // numbers. `shell.exec` would clamp them to `MAX_TIMEOUT_MS`
        // anyway via `.unwrap_or(DEFAULT)`, but flagging them as
        // "absent" here is more honest about the type mismatch.
        let v = json!({ "timeout_ms": -1 });
        assert_eq!(get_optional_u64(&v, "timeout_ms"), None);
    }

    #[test]
    fn get_optional_u64_returns_none_for_non_integral_float() {
        let v = json!({ "timeout_ms": 1.5 });
        assert_eq!(get_optional_u64(&v, "timeout_ms"), None);
    }
}
