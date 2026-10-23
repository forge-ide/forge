//! Event log and search index.
//!
//! M1 has only the subscription hub: it routes published events to every subscription whose
//! topics match. The JSONL log, SQLite index, and `afterSeq` replay land with sessions in M2.

pub mod hub;

pub use hub::{Hub, Subscriber};
