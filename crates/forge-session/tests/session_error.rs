//! F-076: regression tests for the typed `SessionError` returned from
//! `Session::emit`. These assert the *variant shape* — not real disk
//! failures, which on tokio::fs::File backed by an open fd cannot be
//! reliably induced from the outside (chmod doesn't affect open fds,
//! deleting the file doesn't either, /dev/full only fails at create-time,
//! and RLIMIT_FSIZE delivers SIGXFSZ which kills the process).
//!
//! Mirrors the F-074 `ToolError` test style: construct the variant
//! directly via the same conversion path the production code uses
//! (`ForgeError -> SessionError::EventLog{Append,Flush}`) and assert
//! discriminant, `Display`, and `source()`. This is the boundary the
//! change actually introduces — wrapping `EventLog::append`/`flush`'s
//! typed `forge_core::ForgeError` into `SessionError`.

use std::io;

use forge_core::ForgeError;
use forge_session::SessionError;

#[test]
fn event_log_append_wraps_forge_error_and_preserves_source() {
    // Production path: `EventLog::append` returns `forge_core::Result<()>`
    // (i.e. `Result<(), ForgeError>`). `Session::emit` maps the failure
    // into `SessionError::EventLogAppend`. Assert that the `?`-chained
    // source survives so callers wanting to switch on retryable I/O can
    // still drill down to the underlying `io::Error`.
    let inner: ForgeError = io::Error::new(io::ErrorKind::PermissionDenied, "denied").into();
    let err = SessionError::EventLogAppend(inner);

    assert!(matches!(err, SessionError::EventLogAppend(_)));
    assert!(err.to_string().starts_with("event log append failed:"));

    // The forge_core::ForgeError must be reachable via `source()` so a
    // future caller can downcast to the typed inner without parsing the
    // outer Display string.
    let src = std::error::Error::source(&err).expect("source must be exposed");
    let inner_str = src.to_string();
    assert!(
        inner_str.contains("denied"),
        "underlying ForgeError text lost: {inner_str}"
    );
}

#[test]
fn event_log_flush_wraps_forge_error_and_preserves_source() {
    let inner: ForgeError = io::Error::other("disk full").into();
    let err = SessionError::EventLogFlush(inner);

    assert!(matches!(err, SessionError::EventLogFlush(_)));
    assert!(err.to_string().starts_with("event log flush failed:"));

    let src = std::error::Error::source(&err).expect("source must be exposed");
    assert!(src.to_string().contains("disk full"));
}

#[test]
fn append_and_flush_are_distinguishable_variants() {
    // The whole point of F-076: callers must be able to pattern-match
    // on the specific failure mode. If the two variants collapse into
    // a single one (e.g. `EventLogIo` covering both), the typed
    // information needed for retry/escalate decisions is lost again.
    let inner_a: ForgeError = io::Error::new(io::ErrorKind::PermissionDenied, "a").into();
    let inner_b: ForgeError = io::Error::new(io::ErrorKind::PermissionDenied, "b").into();

    let append = SessionError::EventLogAppend(inner_a);
    let flush = SessionError::EventLogFlush(inner_b);

    let is_append = matches!(append, SessionError::EventLogAppend(_));
    let is_flush_not_append = !matches!(flush, SessionError::EventLogAppend(_));
    assert!(is_append);
    assert!(is_flush_not_append);
}

#[test]
fn session_error_is_anyhow_compatible_for_existing_callers() {
    // `orchestrator::run_turn` returns `anyhow::Result<()>` and propagates
    // `session.emit(...).await?`. anyhow has a blanket `From<E>` for any
    // `E: std::error::Error + Send + Sync + 'static`, which thiserror
    // derives for us. This test pins that contract: if someone strips
    // a derive from `SessionError` the orchestrator's `?` would stop
    // compiling, and that breakage should be caught here, not in a
    // distant caller.
    fn require_sync_error<E: std::error::Error + Send + Sync + 'static>(_: &E) {}
    let inner: ForgeError = io::Error::from(io::ErrorKind::PermissionDenied).into();
    let err = SessionError::EventLogAppend(inner);
    require_sync_error(&err);

    let _: anyhow::Error = anyhow::Error::new(err);
}

#[tokio::test]
async fn session_emit_signature_returns_session_error() {
    // Lock the public surface: `Session::emit` must return
    // `Result<(), SessionError>`. A test that compiles is the assertion;
    // if the signature drifts back to `anyhow::Result<()>` this stops
    // compiling, which is the regression we want to catch.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    let session = forge_session::session::Session::create(path).await.unwrap();
    let result: Result<(), SessionError> = session
        .emit(forge_core::Event::SessionEnded {
            at: chrono::Utc::now(),
            reason: forge_core::EndReason::Completed,
            archived: false,
        })
        .await;
    result.unwrap();
}
