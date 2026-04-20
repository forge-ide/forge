//! `forge mcp import` subcommand (F-131).
//!
//! Reads an MCP configuration from one of six source formats and converts
//! it to Forge's universal `.mcp.json`. Dry-run by default (prints a
//! unified diff against the current `.mcp.json`); writes only when
//! `--apply` is passed.
//!
//! The source converters live in [`forge_mcp::import`]; this module is the
//! thin CLI-side wrapper around them.

use anyhow::{anyhow, Context, Result};
use forge_mcp::{
    import::{detect_all, Detection, ImportSource},
    render_universal, McpServerSpec,
};
use similar::TextDiff;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Arguments for `forge mcp import`.
#[derive(Debug, Clone)]
pub struct ImportArgs {
    /// Workspace root. Defaults to the current working directory.
    pub workspace_root: PathBuf,
    /// User home directory. Defaults to `dirs::home_dir()`.
    pub home: PathBuf,
    /// `None` means auto-detect mode.
    pub source: Option<ImportSource>,
    /// Dry-run (print diff) when false; write when true.
    pub apply: bool,
}

/// Entry point for `forge mcp import`. Writes human-readable output to
/// `out` (usually stdout). Returns the exit code the CLI should exit with.
pub fn run(args: &ImportArgs, out: &mut impl Write) -> Result<i32> {
    match args.source {
        Some(source) => run_single(source, args, out),
        None => run_auto(args, out),
    }
}

fn run_single(source: ImportSource, args: &ImportArgs, out: &mut impl Write) -> Result<i32> {
    let path = source
        .default_path(&args.workspace_root, &args.home)
        .ok_or_else(|| anyhow!("cannot resolve default path for source {}", source.slug()))?;
    if !path.exists() {
        writeln!(
            out,
            "no {} config found at {}",
            source.slug(),
            path.display()
        )?;
        return Ok(0);
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("reading {} source at {}", source.slug(), path.display()))?;
    let servers = source
        .convert(&raw)
        .with_context(|| format!("converting {} source at {}", source.slug(), path.display()))?;
    writeln!(
        out,
        "found {} MCP server(s) in {} ({})",
        servers.len(),
        path.display(),
        source.slug()
    )?;
    finalize(&servers, args, out)
}

fn run_auto(args: &ImportArgs, out: &mut impl Write) -> Result<i32> {
    let report = detect_all(&args.workspace_root, &args.home)?;
    if report.found.is_empty() {
        writeln!(out, "no MCP configs detected in any known source location")?;
        return Ok(0);
    }

    writeln!(out, "detected {} source(s):", report.found.len())?;
    for Detection {
        source,
        path,
        servers,
    } in &report.found
    {
        writeln!(
            out,
            "  - {:<8} {} ({} server(s))",
            source.slug(),
            path.display(),
            servers.len()
        )?;
    }

    // Auto-merge policy: walk sources in `ImportSource::all()` order;
    // later sources override earlier ones on name collision. Documented
    // both here and in the CLI `--help` text. We surface collisions to
    // the user so the override isn't silent.
    let mut merged: BTreeMap<String, McpServerSpec> = BTreeMap::new();
    for Detection {
        source, servers, ..
    } in &report.found
    {
        for (name, spec) in servers {
            if merged.insert(name.clone(), spec.clone()).is_some() {
                writeln!(
                    out,
                    "note: {} overrides an earlier definition of {:?}",
                    source.slug(),
                    name
                )?;
            }
        }
    }
    writeln!(out, "merged into {} unique MCP server(s)", merged.len())?;

    finalize(&merged, args, out)
}

fn finalize(
    servers: &BTreeMap<String, McpServerSpec>,
    args: &ImportArgs,
    out: &mut impl Write,
) -> Result<i32> {
    let target = args.workspace_root.join(".mcp.json");
    let existing = if target.exists() {
        fs::read_to_string(&target)
            .with_context(|| format!("reading existing {}", target.display()))?
    } else {
        String::new()
    };
    let proposed = render_universal(servers)?;

    if args.apply {
        fs::write(&target, &proposed).with_context(|| format!("writing {}", target.display()))?;
        writeln!(
            out,
            "wrote {} server(s) to {}",
            servers.len(),
            target.display()
        )?;
    } else {
        let diff = render_diff(&existing, &proposed, &target);
        if diff.is_empty() {
            writeln!(out, "no changes to {}", target.display())?;
        } else {
            writeln!(out, "{}", diff)?;
            writeln!(
                out,
                "dry-run: re-run with --apply to write {}",
                target.display()
            )?;
        }
    }
    Ok(0)
}

/// Produce a unified diff between `old` and `new`. Empty string when the
/// two sides are identical — callers treat that as "no changes".
pub(crate) fn render_diff(old: &str, new: &str, target: &Path) -> String {
    if old == new {
        return String::new();
    }
    let diff = TextDiff::from_lines(old, new);
    let header_old = if old.is_empty() {
        "(missing)".to_string()
    } else {
        target.display().to_string()
    };
    let header_new = format!("{} (proposed)", target.display());
    diff.unified_diff()
        .header(&header_old, &header_new)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_file(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    #[test]
    fn dry_run_single_source_prints_diff_and_does_not_write() {
        let ws = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        write_file(
            home.path(),
            ".cursor/mcp.json",
            r#"{ "mcpServers": { "gh": { "command": "npx", "args": ["-y", "server-github"] } } }"#,
        );

        let args = ImportArgs {
            workspace_root: ws.path().to_path_buf(),
            home: home.path().to_path_buf(),
            source: Some(ImportSource::Cursor),
            apply: false,
        };
        let mut buf = Vec::new();
        let code = run(&args, &mut buf).unwrap();
        assert_eq!(code, 0);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("found 1 MCP server"), "output: {output}");
        assert!(output.contains("dry-run"), "output: {output}");
        assert!(
            output.contains("+  \"mcpServers\""),
            "diff expected: {output}"
        );
        assert!(!ws.path().join(".mcp.json").exists());
    }

    #[test]
    fn apply_writes_universal_config_and_reports_count() {
        let ws = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        write_file(
            home.path(),
            ".cursor/mcp.json",
            r#"{ "mcpServers": { "gh": { "command": "npx" } } }"#,
        );
        let args = ImportArgs {
            workspace_root: ws.path().to_path_buf(),
            home: home.path().to_path_buf(),
            source: Some(ImportSource::Cursor),
            apply: true,
        };
        let mut buf = Vec::new();
        let code = run(&args, &mut buf).unwrap();
        assert_eq!(code, 0);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("wrote 1 server"), "output: {output}");
        let written = fs::read_to_string(ws.path().join(".mcp.json")).unwrap();
        assert!(written.contains("\"gh\""), "written: {written}");
        assert!(
            written.contains("\"command\": \"npx\""),
            "written: {written}"
        );
    }

    #[test]
    fn auto_detect_lists_all_sources_and_merges() {
        let ws = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        write_file(
            ws.path(),
            ".vscode/mcp.json",
            r#"{ "servers": { "shared": { "command": "vs" }, "vs-only": { "command": "vsx" } } }"#,
        );
        write_file(
            home.path(),
            ".cursor/mcp.json",
            r#"{ "mcpServers": { "shared": { "command": "cur" }, "cursor-only": { "command": "curx" } } }"#,
        );
        let args = ImportArgs {
            workspace_root: ws.path().to_path_buf(),
            home: home.path().to_path_buf(),
            source: None,
            apply: false,
        };
        let mut buf = Vec::new();
        run(&args, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("detected 2 source"), "output: {output}");
        assert!(output.contains("vscode"), "output: {output}");
        assert!(output.contains("cursor"), "output: {output}");
        // Later source (cursor, walked after vscode) wins on collision.
        assert!(output.contains("overrides"), "output: {output}");
        assert!(output.contains("merged into 3"), "output: {output}");
    }

    #[test]
    fn auto_detect_reports_when_nothing_found() {
        let ws = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let args = ImportArgs {
            workspace_root: ws.path().to_path_buf(),
            home: home.path().to_path_buf(),
            source: None,
            apply: false,
        };
        let mut buf = Vec::new();
        run(&args, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(
            output.contains("no MCP configs detected"),
            "output: {output}"
        );
    }

    #[test]
    fn missing_single_source_reports_cleanly_not_error() {
        let ws = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let args = ImportArgs {
            workspace_root: ws.path().to_path_buf(),
            home: home.path().to_path_buf(),
            source: Some(ImportSource::Kiro),
            apply: false,
        };
        let mut buf = Vec::new();
        let code = run(&args, &mut buf).unwrap();
        assert_eq!(code, 0);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("no kiro config found"), "output: {output}");
    }

    #[test]
    fn apply_with_existing_file_overwrites() {
        let ws = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        write_file(
            ws.path(),
            ".mcp.json",
            r#"{ "mcpServers": { "stale": { "command": "old" } } }"#,
        );
        write_file(
            home.path(),
            ".cursor/mcp.json",
            r#"{ "mcpServers": { "fresh": { "command": "new" } } }"#,
        );
        let args = ImportArgs {
            workspace_root: ws.path().to_path_buf(),
            home: home.path().to_path_buf(),
            source: Some(ImportSource::Cursor),
            apply: true,
        };
        let mut buf = Vec::new();
        run(&args, &mut buf).unwrap();
        let written = fs::read_to_string(ws.path().join(".mcp.json")).unwrap();
        assert!(!written.contains("stale"), "written: {written}");
        assert!(written.contains("fresh"), "written: {written}");
    }

    #[test]
    fn render_diff_is_empty_for_identical_strings() {
        let d = render_diff("same\n", "same\n", Path::new("/tmp/.mcp.json"));
        assert!(d.is_empty());
    }
}
