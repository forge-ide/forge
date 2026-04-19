/// Integration tests for CLI argument parsing.
/// These import the production `Cli` struct directly so any structural change
/// to the real command hierarchy will break these tests.
use clap::Parser;
use forge_cli::{Cli, Commands, RunCommands, SessionCommands, SessionNewKind};
use std::path::PathBuf;

#[test]
fn parse_session_new_agent_with_workspace() {
    let cli = Cli::try_parse_from([
        "forge",
        "session",
        "new",
        "agent",
        "code-review",
        "--workspace",
        "/tmp/myproject",
    ])
    .expect("should parse");
    let Commands::Session {
        cmd:
            SessionCommands::New {
                kind:
                    SessionNewKind::Agent {
                        name, workspace, ..
                    },
            },
    } = cli.command
    else {
        panic!("wrong command shape");
    };
    assert_eq!(name, "code-review");
    assert_eq!(workspace, Some(PathBuf::from("/tmp/myproject")));
}

#[test]
fn parse_session_new_agent_without_workspace() {
    let cli =
        Cli::try_parse_from(["forge", "session", "new", "agent", "helper"]).expect("should parse");
    let Commands::Session {
        cmd:
            SessionCommands::New {
                kind:
                    SessionNewKind::Agent {
                        name, workspace, ..
                    },
            },
    } = cli.command
    else {
        panic!("wrong command shape");
    };
    assert_eq!(name, "helper");
    assert!(workspace.is_none());
}

#[test]
fn parse_session_new_agent_with_provider_flag() {
    let cli = Cli::try_parse_from([
        "forge",
        "session",
        "new",
        "agent",
        "code-review",
        "--provider",
        "ollama:qwen2.5:0.5b",
    ])
    .expect("should parse");
    let Commands::Session {
        cmd:
            SessionCommands::New {
                kind: SessionNewKind::Agent { provider, .. },
            },
    } = cli.command
    else {
        panic!("wrong command shape");
    };
    assert_eq!(provider, Some("ollama:qwen2.5:0.5b".to_string()));
}

#[test]
fn parse_session_new_provider() {
    let cli = Cli::try_parse_from([
        "forge",
        "session",
        "new",
        "provider",
        "anthropic/claude-opus-4",
        "--workspace",
        "/home/user/proj",
    ])
    .expect("should parse");
    let Commands::Session {
        cmd:
            SessionCommands::New {
                kind: SessionNewKind::Provider { spec, workspace },
            },
    } = cli.command
    else {
        panic!("wrong command shape");
    };
    assert_eq!(spec, "anthropic/claude-opus-4");
    assert_eq!(workspace, Some(PathBuf::from("/home/user/proj")));
}

#[test]
fn parse_session_list() {
    let cli = Cli::try_parse_from(["forge", "session", "list"]).expect("should parse");
    assert!(matches!(
        cli.command,
        Commands::Session {
            cmd: SessionCommands::List
        }
    ));
}

#[test]
fn parse_session_tail() {
    let cli = Cli::try_parse_from(["forge", "session", "tail", "abc123def4560000"])
        .expect("should parse");
    let Commands::Session {
        cmd: SessionCommands::Tail { id },
    } = cli.command
    else {
        panic!("wrong command shape");
    };
    assert_eq!(id, "abc123def4560000");
}

#[test]
fn parse_session_kill() {
    let cli = Cli::try_parse_from(["forge", "session", "kill", "deadbeefcafe0000"])
        .expect("should parse");
    let Commands::Session {
        cmd: SessionCommands::Kill { id },
    } = cli.command
    else {
        panic!("wrong command shape");
    };
    assert_eq!(id, "deadbeefcafe0000");
}

/// F-057 (T12a): clap must reject a path-traversal session id during
/// argument parsing — before any command body runs — so the filesystem
/// is never touched with an attacker-controlled path component.
#[test]
fn parse_session_kill_rejects_path_traversal() {
    let result = Cli::try_parse_from(["forge", "session", "kill", "../../tmp/x"]);
    assert!(
        result.is_err(),
        "path-traversal session id must be rejected at parse time"
    );
}

#[test]
fn parse_session_tail_rejects_path_traversal() {
    let result = Cli::try_parse_from(["forge", "session", "tail", "../../tmp/x"]);
    assert!(
        result.is_err(),
        "path-traversal session id must be rejected at parse time"
    );
}

#[test]
fn parse_session_kill_rejects_absolute_path() {
    let result = Cli::try_parse_from(["forge", "session", "kill", "/etc/passwd"]);
    assert!(result.is_err(), "absolute path id must be rejected");
}

#[test]
fn parse_session_kill_rejects_uppercase_hex() {
    // SessionId::new() emits lowercase hex only; uppercase cannot have
    // come from the canonical generator.
    let result = Cli::try_parse_from(["forge", "session", "kill", "DEADBEEFCAFEBABE"]);
    assert!(result.is_err(), "uppercase hex id must be rejected");
}

#[test]
fn parse_session_kill_rejects_wrong_length() {
    // 15 chars — one short of the canonical 16.
    let result = Cli::try_parse_from(["forge", "session", "kill", "deadbeefcafebab"]);
    assert!(result.is_err(), "15-char id must be rejected");
}

#[test]
fn parse_session_kill_rejects_non_hex() {
    // 16 chars but contains 'g'.
    let result = Cli::try_parse_from(["forge", "session", "kill", "deadbeefcafebabg"]);
    assert!(result.is_err(), "non-hex id must be rejected");
}

#[test]
fn parse_run_agent_with_default_input() {
    let cli = Cli::try_parse_from(["forge", "run", "agent", "code-review"]).expect("should parse");
    let Commands::Run {
        cmd: RunCommands::Agent { name, input },
    } = cli.command
    else {
        panic!("wrong command shape");
    };
    assert_eq!(name, "code-review");
    assert_eq!(input, "-");
}

#[test]
fn parse_run_agent_with_file_input() {
    let cli = Cli::try_parse_from([
        "forge",
        "run",
        "agent",
        "helper",
        "--input",
        "/tmp/prompt.txt",
    ])
    .expect("should parse");
    let Commands::Run {
        cmd: RunCommands::Agent { name, input },
    } = cli.command
    else {
        panic!("wrong command shape");
    };
    assert_eq!(name, "helper");
    assert_eq!(input, "/tmp/prompt.txt");
}

#[test]
fn unknown_command_returns_error() {
    let result = Cli::try_parse_from(["forge", "bogus"]);
    assert!(result.is_err(), "bogus command should fail to parse");
}
