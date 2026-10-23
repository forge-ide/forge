//! Event log and search index.
//!
//! [`EventLog`] appends each session's events to a JSONL file, the source of truth, and indexes
//! them in SQLite for replay after a stream cursor and for search. [`Hub`] routes published
//! events to every subscription whose topics match.

pub mod hub;
pub mod log;

pub use hub::{Hub, Subscriber};
pub use log::{
    EventLog, LogError, NoRedaction, Query, Redactor, Replayed, SharedConn, log_path, topics,
};
