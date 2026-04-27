//! Thin re-export shim for the SSRF guard.
//!
//! The implementation lives in [`forge_core::url_safety`]. F-585 lifted the
//! original F-346 function out of this crate so `forge-providers` can reuse
//! the same guard against custom OpenAI-compatible base URLs without
//! pulling in a wrong-direction dep on `forge-mcp`. Existing call sites
//! (`super::http`, the integration test under `tests/`, and the UAT script)
//! keep working unchanged via this re-export.

pub use forge_core::url_safety::check_url;
