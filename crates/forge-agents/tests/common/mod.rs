//! Shared tracing-capture subscriber for integration tests.
//!
//! Tests drain a global `MakeWriter` buffer and scan for formatted output.
//! The subscriber is installed exactly once; a mutex serializes tests that
//! read the buffer so concurrent test threads don't interleave assertions.

use std::{
    io,
    sync::{Mutex as StdMutex, Once, OnceLock},
};

use tracing_subscriber::fmt::MakeWriter;

fn capture_buf() -> &'static StdMutex<Vec<u8>> {
    static BUF: OnceLock<StdMutex<Vec<u8>>> = OnceLock::new();
    BUF.get_or_init(|| StdMutex::new(Vec::new()))
}

/// Lock held for the duration of a capture-reading test. Other tracing
/// tests block on this so the drained buffer is attributable to one test.
pub fn capture_test_lock() -> &'static StdMutex<()> {
    static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| StdMutex::new(()))
}

pub struct CaptureWriter;
impl io::Write for CaptureWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        capture_buf().lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl<'a> MakeWriter<'a> for CaptureWriter {
    type Writer = CaptureWriter;
    fn make_writer(&'a self) -> Self::Writer {
        CaptureWriter
    }
}

/// Install a fmt subscriber that writes every event (TRACE and up) into the
/// shared buffer. Subsequent calls are no-ops.
pub fn install_capture_subscriber() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_target(true)
            .with_writer(CaptureWriter)
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("install capture subscriber");
    });
}

/// Drain the shared buffer and return the captured output as UTF-8.
pub fn drain_capture() -> String {
    let mut buf = capture_buf().lock().unwrap();
    let out = String::from_utf8(buf.clone()).expect("utf-8 logs");
    buf.clear();
    out
}
