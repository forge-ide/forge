//! Fixture-based integration tests for `forge_mcp::import` (F-131).
//!
//! Each DoD-mandated source format gets a representative sample under
//! `tests/fixtures/import/`. The test reads the fixture, runs it through
//! the matching converter, and checks the converted `McpServerSpec` map
//! against the universal-schema expected shape.

use forge_mcp::{
    import::{claude, codex, continue_, cursor, kiro, vscode},
    McpServerSpec, ServerKind,
};
use std::{collections::BTreeMap, fs, path::PathBuf};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("import")
}

fn read(name: &str) -> String {
    fs::read_to_string(fixtures_dir().join(name))
        .unwrap_or_else(|e| panic!("reading fixture {name}: {e}"))
}

fn stdio_spec(command: &str, args: &[&str], env: &[(&str, &str)]) -> McpServerSpec {
    McpServerSpec {
        kind: ServerKind::Stdio {
            command: command.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        },
    }
}

fn http_spec(url: &str, headers: &[(&str, &str)]) -> McpServerSpec {
    McpServerSpec {
        kind: ServerKind::Http {
            url: url.into(),
            headers: headers
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        },
    }
}

#[test]
fn vscode_fixture_matches_expected_universal_shape() {
    let got = vscode::convert(&read("vscode_mcp.json")).unwrap();

    let mut want: BTreeMap<String, McpServerSpec> = BTreeMap::new();
    want.insert(
        "playwright".into(),
        stdio_spec("npx", &["-y", "@microsoft/mcp-server-playwright"], &[]),
    );
    want.insert(
        "github".into(),
        http_spec("https://api.githubcopilot.com/mcp", &[]),
    );

    assert_eq!(got, want);
}

#[test]
fn cursor_fixture_matches_expected_universal_shape() {
    let got = cursor::convert(&read("cursor_mcp.json")).unwrap();

    let mut want: BTreeMap<String, McpServerSpec> = BTreeMap::new();
    want.insert(
        "filesystem".into(),
        stdio_spec(
            "npx",
            &["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
            &[("FS_ROOT", "/tmp")],
        ),
    );
    want.insert(
        "remote".into(),
        http_spec(
            "https://mcp.example.com/api",
            &[("Authorization", "Bearer ${env:MCP_TOKEN}")],
        ),
    );

    assert_eq!(got, want);
}

#[test]
fn claude_desktop_fixture_matches_expected_universal_shape() {
    let got = claude::convert(&read("claude_desktop_config.json")).unwrap();

    let mut want: BTreeMap<String, McpServerSpec> = BTreeMap::new();
    want.insert(
        "filesystem".into(),
        stdio_spec(
            "npx",
            &[
                "-y",
                "@modelcontextprotocol/server-filesystem",
                "/Users/example/Desktop",
            ],
            &[],
        ),
    );
    want.insert(
        "github".into(),
        stdio_spec(
            "npx",
            &["-y", "@modelcontextprotocol/server-github"],
            &[("GITHUB_PERSONAL_ACCESS_TOKEN", "ghp_fake")],
        ),
    );

    assert_eq!(got, want);
}

#[test]
fn continue_fixture_matches_expected_universal_shape() {
    let got = continue_::convert(&read("continue_config.json")).unwrap();

    let mut want: BTreeMap<String, McpServerSpec> = BTreeMap::new();
    want.insert(
        "sqlite".into(),
        stdio_spec("npx", &["-y", "mcp-sqlite", "/tmp/app.db"], &[]),
    );
    want.insert(
        "remote-sse".into(),
        http_spec("https://continue.example/mcp", &[]),
    );

    assert_eq!(got, want);
}

#[test]
fn kiro_fixture_matches_expected_universal_shape() {
    let got = kiro::convert(&read("kiro_mcp.json")).unwrap();

    let mut want: BTreeMap<String, McpServerSpec> = BTreeMap::new();
    want.insert(
        "web-search".into(),
        stdio_spec(
            "npx",
            &["-y", "@modelcontextprotocol/server-bravesearch"],
            &[("BRAVE_API_KEY", "${BRAVE_API_KEY}")],
        ),
    );

    assert_eq!(got, want);
}

#[test]
fn codex_fixture_matches_expected_universal_shape() {
    let got = codex::convert(&read("codex_config.toml")).unwrap();

    let mut want: BTreeMap<String, McpServerSpec> = BTreeMap::new();
    want.insert(
        "docs".into(),
        stdio_spec("docs-server", &["--port", "4242"], &[("LOG_LEVEL", "info")]),
    );
    want.insert(
        "github".into(),
        stdio_spec(
            "npx",
            &["-y", "@modelcontextprotocol/server-github"],
            &[("GITHUB_PERSONAL_ACCESS_TOKEN", "ghp_fake")],
        ),
    );

    assert_eq!(got, want);
}
