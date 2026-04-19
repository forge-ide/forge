//! Typed errors for the session crate.
//!
//! F-076: `Session::emit` previously wrapped every `EventLog::append` /
//! `EventLog::flush` failure in `anyhow::anyhow!(e)`, erasing the typed
//! `forge_core::ForgeError` underneath and forcing top-level callers to
//! match on Display strings to distinguish retryable I/O from fatal
//! state. Surfacing a typed enum lets callers (e.g. orchestrator,
//! server loop) decide on retry / escalate per failure mode.
//!
//! Variants intentionally keep `forge_core::ForgeError` as the inner
//! type rather than collapsing to `std::io::Error` — `EventLog`'s
//! return type already carries Io / Serde / Other discrimination, and
//! downcasting at the boundary would silently drop the Serde and Other
//! cases.

use forge_core::ForgeError;

/// Error returned by [`crate::session::Session::emit`].
///
/// The two `EventLog*` variants share an inner type but represent
/// distinct failure modes: an `append` failure means the event was
/// never durably staged, while a `flush` failure means the event was
/// staged in the buffer but the durable write never completed. Callers
/// that retry should distinguish these cases — re-emitting after a
/// flush failure risks duplicate events on the next successful flush.
///
/// No `#[from]` impl is provided because both variants wrap the same
/// inner type; the caller (`Session::emit`) selects the variant
/// explicitly via `.map_err(SessionError::EventLogAppend)` /
/// `.map_err(SessionError::EventLogFlush)`.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// `EventLog::append` failed before the event was buffered.
    /// Inner is the typed `forge_core::ForgeError` so callers can
    /// match on `ForgeError::Io` (e.g. `ErrorKind::PermissionDenied`)
    /// without parsing the outer Display.
    #[error("event log append failed: {0}")]
    EventLogAppend(#[source] ForgeError),

    /// `EventLog::flush` failed after the event was buffered. The event
    /// may or may not be durable — treat as ambiguous and surface to
    /// the operator rather than silently retrying.
    #[error("event log flush failed: {0}")]
    EventLogFlush(#[source] ForgeError),
}
