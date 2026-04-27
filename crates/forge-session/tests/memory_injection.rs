//! F-601: integration tests for cross-session memory injection.
//!
//! These tests cover the seam that `serve_with_session` uses — they
//! exercise the public `forge_agents` API (`MemoryStore` + frontmatter +
//! `assemble_system_prompt`) that the server composes once per session.
//! End-to-end coverage of the daemon path is deferred to the existing
//! orchestrator-level AGENTS.md test, which is the precedent for
//! per-turn system-prompt assertions.

use forge_agents::{
    assemble_system_prompt, Memory, MemoryFrontmatter, MemoryStore, WriteMode, MEMORY_HEADING,
};
use tempfile::tempdir;

#[test]
fn memory_body_injects_after_agents_md_under_memory_heading() {
    let agents_md_prefix = "\n\n---\nAGENTS.md (workspace):\nworkspace rules";
    let memory_body = "remember: ship Phase 3 by Friday";

    let assembled = assemble_system_prompt(Some(agents_md_prefix), Some(memory_body))
        .expect("both inputs present must yield Some");

    let agents_idx = assembled
        .find("AGENTS.md (workspace):")
        .expect("AGENTS.md label must appear");
    let mem_idx = assembled
        .find("## Memory")
        .expect("Memory heading must appear");
    assert!(
        agents_idx < mem_idx,
        "AGENTS.md must precede Memory heading; got: {assembled}"
    );
    assert!(
        assembled.ends_with(memory_body),
        "memory body must close out the prompt; got: {assembled}"
    );
    assert!(
        assembled.contains(MEMORY_HEADING.trim()),
        "MEMORY_HEADING must be present"
    );
}

#[test]
fn memory_alone_still_uses_memory_heading() {
    let memory_body = "no agents.md, just memory";
    let assembled = assemble_system_prompt(None, Some(memory_body)).unwrap();
    assert!(assembled.contains("## Memory"));
    assert!(assembled.ends_with(memory_body));
}

#[test]
fn no_memory_no_agents_md_yields_none() {
    assert!(assemble_system_prompt(None, None).is_none());
}

#[test]
fn write_then_read_round_trips_for_a_named_agent() {
    let dir = tempdir().unwrap();
    let store = MemoryStore::new(dir.path());
    let result = store
        .write("scribe", "phase 3 plan", WriteMode::Append)
        .unwrap();
    assert_eq!(result.frontmatter.version, 1);

    // A second session of the same agent observes the prior write.
    let reread = store.load("scribe").unwrap().unwrap();
    assert_eq!(reread.body, "phase 3 plan");
    assert_eq!(reread.frontmatter.version, 1);
}

#[test]
fn missing_file_does_not_crash_load_path() {
    let dir = tempdir().unwrap();
    let store = MemoryStore::new(dir.path());
    assert!(store.load("never-written").unwrap().is_none());
}

#[test]
fn explicit_save_with_chosen_metadata_round_trips() {
    let dir = tempdir().unwrap();
    let store = MemoryStore::new(dir.path());
    let memory = Memory {
        frontmatter: MemoryFrontmatter {
            updated_at: chrono::Utc::now(),
            version: 42,
        },
        body: "frozen body".to_string(),
    };
    store.save("scribe", &memory).unwrap();

    let reread = store.load("scribe").unwrap().unwrap();
    assert_eq!(reread.frontmatter.version, 42);
    assert_eq!(reread.body, "frozen body");
}
