use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use forge_core::{Event, EventLog};
use tokio::sync::{broadcast, Mutex};

use crate::error::SessionError;

pub struct Session {
    pub log_path: PathBuf,
    pub event_tx: broadcast::Sender<(u64, Event)>,
    log: Arc<Mutex<EventLog>>,
    seq: Arc<Mutex<u64>>,
    /// F-598: tripped while [`crate::compaction::compact`] is running so the
    /// orchestrator's auto-trigger never re-enters compaction during the
    /// privileged summary call. The summary stream emits events that
    /// re-enter the same session log; without the guard a misbehaving
    /// provider that drives the byte budget over the threshold mid-summary
    /// could fire a second compaction concurrently.
    compacting: Arc<AtomicBool>,
}

impl Session {
    pub async fn create(log_path: PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = log_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let log = EventLog::create(&log_path)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        let (tx, _) = broadcast::channel(1024);
        Ok(Self {
            log_path,
            event_tx: tx,
            log: Arc::new(Mutex::new(log)),
            seq: Arc::new(Mutex::new(0)),
            compacting: Arc::new(AtomicBool::new(false)),
        })
    }

    /// F-598: returns `true` if a [`crate::compaction::compact`] pass has
    /// claimed the guard slot, `false` if one was already in flight. The
    /// caller MUST pair a successful claim with [`Self::release_compacting`]
    /// (typically via a guard struct) so a panic mid-compaction doesn't
    /// strand the flag set forever.
    pub fn try_claim_compacting(&self) -> bool {
        self.compacting
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// F-598: drop the in-flight compaction marker. Idempotent — calling on
    /// a clear flag is a no-op.
    pub fn release_compacting(&self) {
        self.compacting.store(false, Ordering::SeqCst);
    }

    /// F-598: observe the in-flight compaction flag without claiming it.
    /// Used by the orchestrator's auto-trigger to skip a second pass while
    /// one is already running.
    pub fn is_compacting(&self) -> bool {
        self.compacting.load(Ordering::SeqCst)
    }

    /// Append `event` to the durable event log and broadcast it to
    /// subscribers.
    ///
    /// F-076: returns the typed [`SessionError`] so callers can
    /// distinguish an append failure (event never staged) from a flush
    /// failure (event staged but durability uncertain). The broadcast
    /// `send` failure is intentionally swallowed — `broadcast::Sender`
    /// returns `Err` when zero receivers are subscribed, which is the
    /// normal warmup state and not an error condition.
    pub async fn emit(&self, event: Event) -> Result<(), SessionError> {
        let mut seq = self.seq.lock().await;
        *seq += 1;
        let seq_num = *seq;

        let mut log = self.log.lock().await;
        log.append(&event)
            .await
            .map_err(SessionError::EventLogAppend)?;
        log.flush().await.map_err(SessionError::EventLogFlush)?;
        drop(log);
        drop(seq);

        let _ = self.event_tx.send((seq_num, event));
        Ok(())
    }

    pub async fn current_seq(&self) -> u64 {
        *self.seq.lock().await
    }
}
