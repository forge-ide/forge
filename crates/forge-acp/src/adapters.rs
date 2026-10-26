//! The adapter registry: one manifest per agent Forge knows, with how to install it, the
//! version Forge is tested with, how it signs in, and its known quirks.
//!
//! Manifests live in `crates/forge-acp/adapters/` and are compiled in. They are data; what an
//! agent needs written before it starts (its own permission settings) is its shim, in
//! [`crate::shim`].

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

const MANIFESTS: &[(&str, &str)] = &[
    ("claude-code", include_str!("../adapters/claude-code.toml")),
    ("codex", include_str!("../adapters/codex.toml")),
    ("gemini", include_str!("../adapters/gemini.toml")),
    ("antigravity", include_str!("../adapters/antigravity.toml")),
];

/// One agent's manifest.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Registry key, as profiles name it in `agent`.
    pub id: String,
    pub name: String,
    /// The ID of the built-in profile that runs this agent.
    pub profile: String,
    pub package: Package,
    pub launch: Launch,
    pub auth: Auth,
    pub permissions: Permissions,
    /// Hosts the agent reaches itself, through the egress proxy: its own sign-in and model
    /// endpoints, when it keeps its own login rather than one the proxy injects.
    #[serde(default)]
    pub network: Network,
    pub quirks: Quirks,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Network {
    pub allow: Vec<String>,
}

/// Where the agent comes from.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    /// The npm package, run under `runtime`; without one, `archive`.
    #[serde(default)]
    pub npm: Option<String>,
    /// The exact version Forge is tested with.
    pub version: String,
    /// The program the package installs.
    pub bin: String,
    #[serde(default)]
    pub runtime: Option<Runtime>,
    /// A download unpacked into a fixed directory and run where it is.
    #[serde(default)]
    pub archive: Option<Archive>,
}

/// A pinned download, unpacked into `<home>/<dir>/<version>/`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Archive {
    /// Relative to the home dir.
    pub dir: String,
    /// The zip for Linux on x86-64, and its SHA-256; other platforms have none.
    pub linux_x86_64: Option<String>,
    pub linux_x86_64_sha256: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Runtime {
    /// A Node.js script, run by the `node` found through `PATH`.
    Node,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    /// Arguments after the program.
    pub args: Vec<String>,
    /// Variables the agent always gets.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    /// Its processes talk to each other over loopback ports they pick, which only the
    /// Standard tier on Linux allows (its own network namespace); every other tier lets a
    /// sandboxed process connect only to the egress proxy.
    #[serde(default)]
    pub loopback: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Auth {
    /// The keyring secrets the agent signs in with, injected through the egress proxy.
    pub secrets: Vec<String>,
    /// The agent's own subscription sign-in, which Forge carries when the keyring has none
    /// of `secrets`.
    #[serde(default)]
    pub login: Option<crate::logins::LoginKind>,
    /// The login the agent keeps and refreshes itself, relative to the home dir: signed in
    /// with its own ACP method (`agent.signIn`), and shown to its sessions read-write.
    #[serde(default)]
    pub own_login: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// Where an execute permission request carries its command.
    pub command: CommandSource,
}

/// Where an agent's execute permission requests carry the command they ask about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommandSource {
    /// `rawInput.argv` or `rawInput.command`: an argv array as given, a string as a shell line.
    #[default]
    RawInput,
    /// The request's title is the shell line.
    Title,
    /// The `rawInput.command` of the tool call the request belongs to, as first reported: the
    /// agent's whole argv, shell-quoted.
    ToolCallArgv,
    /// `rawInput.CommandLine`, a shell line (Antigravity's).
    CommandLine,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quirks {
    pub notes: Vec<String>,
}

impl Manifest {
    /// The command that installs the pinned version.
    pub fn install_command(&self) -> Vec<String> {
        if let Some(archive) = &self.package.archive {
            let pinned = (&archive.linux_x86_64, &archive.linux_x86_64_sha256);
            return vec![match pinned {
                (Some(url), Some(sha))
                    if cfg!(all(target_os = "linux", target_arch = "x86_64")) =>
                {
                    let dir = format!("~/{}/{}", archive.dir, self.package.version);
                    format!(
                        "mkdir -p {dir} && curl -fL {url} -o {dir}.zip && echo '{sha}  {dir}.zip' | sha256sum -c && unzip -o {dir}.zip -d {dir} && rm {dir}.zip"
                    )
                }
                _ => format!("{} isn't available for this platform", self.name),
            }];
        }
        vec![
            "npm".into(),
            "install".into(),
            "-g".into(),
            format!(
                "{}@{}",
                self.package.npm.as_deref().unwrap_or_default(),
                self.package.version
            ),
        ]
    }

    /// Where an archive package is unpacked, under `home`.
    pub fn archive_dir(&self, home: &Path) -> Option<PathBuf> {
        let archive = self.package.archive.as_ref()?;
        Some(home.join(&archive.dir).join(&self.package.version))
    }

    /// The command that starts the agent speaking ACP, unless a profile names another.
    pub fn default_command(&self) -> Vec<String> {
        std::iter::once(self.package.bin.clone())
            .chain(self.launch.args.iter().cloned())
            .collect()
    }
}

/// Every manifest Forge ships.
pub fn manifests() -> &'static [Manifest] {
    static PARSED: OnceLock<Vec<Manifest>> = OnceLock::new();
    PARSED.get_or_init(|| {
        MANIFESTS
            .iter()
            .map(|(name, text)| {
                toml::from_str(text).unwrap_or_else(|e| panic!("adapters/{name}.toml: {e}"))
            })
            .collect()
    })
}

/// The manifest for registry key `id`.
pub fn manifest(id: &str) -> Option<&'static Manifest> {
    manifests().iter().find(|m| m.id == id)
}

/// The manifest whose built-in profile is `profile`.
pub fn by_profile(profile: &str) -> Option<&'static Manifest> {
    manifests().iter().find(|m| m.profile == profile)
}

/// How to run an agent command inside the sandbox.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchPlan {
    /// The argv to run, starting with an absolute program path.
    pub argv: Vec<String>,
    /// What the sandbox must show read-only for it to run.
    pub read_only: Vec<PathBuf>,
    /// The installed version, when Forge can read it without running anything.
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LaunchError {
    #[error("{0} is not installed (not found in PATH)")]
    NotInstalled(String),
    #[error("{0} is not an executable file")]
    NotExecutable(String),
    #[error("{0} needs node, which is not installed (not found in PATH)")]
    NoRuntime(String),
}

/// How to run `command` in a sandbox that shows nothing of the user's home. A program from the
/// manifest's own npm package runs under `node`, with node and that package shown read-only;
/// any other program must be an executable file and is shown by itself. Nothing is run to
/// find out: a profile file in a cloned repo can name any command, and the daemon runs
/// nothing outside the sandbox on its word.
pub fn plan(
    manifest: Option<&Manifest>,
    command: &[String],
    path: Option<&OsStr>,
) -> Result<LaunchPlan, LaunchError> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    plan_in(manifest, command, path, home.as_deref())
}

/// [`plan`], with archive packages unpacked under `home`.
fn plan_in(
    manifest: Option<&Manifest>,
    command: &[String],
    path: Option<&OsStr>,
    home: Option<&Path>,
) -> Result<LaunchPlan, LaunchError> {
    let (first, rest) = command
        .split_first()
        .ok_or_else(|| LaunchError::NotInstalled(String::new()))?;
    // An archive's program runs from where it was unpacked, with its whole version dir (its
    // helper binaries) shown read-only.
    if let Some(manifest) = manifest.filter(|m| m.package.archive.is_some())
        && *first == manifest.package.bin
    {
        let dir = home
            .and_then(|h| manifest.archive_dir(h))
            .and_then(|d| std::fs::canonicalize(d).ok())
            .ok_or_else(|| LaunchError::NotInstalled(first.clone()))?;
        let program = dir.join(first);
        if !is_executable(&program) {
            return Err(LaunchError::NotInstalled(program.display().to_string()));
        }
        let argv = std::iter::once(program.display().to_string())
            .chain(rest.iter().cloned())
            .collect();
        return Ok(LaunchPlan {
            argv,
            read_only: vec![dir],
            version: Some(manifest.package.version.clone()),
        });
    }
    let program = locate(first, path).ok_or_else(|| LaunchError::NotInstalled(first.clone()))?;
    let package = manifest.and_then(|m| {
        let root = package_root(&program, m.package.npm.as_deref()?)?;
        Some((m, root))
    });
    match package {
        Some((manifest, (root, version))) => match manifest.package.runtime {
            None => Err(LaunchError::NotExecutable(program.display().to_string())),
            Some(Runtime::Node) => {
                let node = locate("node", path)
                    .filter(|p| is_executable(p))
                    .ok_or_else(|| LaunchError::NoRuntime(first.clone()))?;
                let argv = [node.display().to_string(), program.display().to_string()]
                    .into_iter()
                    .chain(rest.iter().cloned())
                    .collect();
                Ok(LaunchPlan {
                    argv,
                    read_only: vec![node, root],
                    version,
                })
            }
        },
        None => {
            if !is_executable(&program) {
                return Err(LaunchError::NotExecutable(program.display().to_string()));
            }
            let argv = std::iter::once(program.display().to_string())
                .chain(rest.iter().cloned())
                .collect();
            Ok(LaunchPlan {
                argv,
                read_only: vec![program],
                version: None,
            })
        }
    }
}

/// `program` as an absolute, canonical path to a regular file, looked up in the absolute
/// directories of `path` when it has no slash.
pub fn locate(program: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    let found = if program.contains('/') {
        PathBuf::from(program)
    } else {
        std::env::split_paths(path?)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())?
    };
    std::fs::canonicalize(found).ok().filter(|p| p.is_file())
}

/// The npm package `script` belongs to and its version, if its nearest `package.json` names
/// package `name`.
fn package_root(script: &Path, name: &str) -> Option<(PathBuf, Option<String>)> {
    #[derive(Deserialize)]
    struct PackageJson {
        name: Option<String>,
        version: Option<String>,
    }
    let dir = script
        .ancestors()
        .skip(1)
        .find(|dir| dir.join("package.json").is_file())?;
    let text = std::fs::read_to_string(dir.join("package.json")).ok()?;
    let json: PackageJson = serde_json::from_str(&text).ok()?;
    (json.name.as_deref() == Some(name)).then(|| (dir.to_path_buf(), json.version))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn an_archive_runs_from_its_unpacked_version_dir() {
        let home = tempfile::tempdir().unwrap();
        let m = manifest("antigravity").unwrap();
        let plan = |home: &Path| plan_in(Some(m), &m.default_command(), None, Some(home));
        assert!(matches!(
            plan(home.path()),
            Err(LaunchError::NotInstalled(_))
        ));
        let dir = m.archive_dir(home.path()).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join(&m.package.bin);
        std::fs::write(&program, "#!/bin/sh\n").unwrap();
        // Not executable yet.
        assert!(plan(home.path()).is_err());
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let got = plan(home.path()).unwrap();
        let dir = std::fs::canonicalize(&dir).unwrap();
        assert_eq!(got.argv, [dir.join(&m.package.bin).display().to_string()]);
        // The whole version dir, which holds its helper binary too.
        assert_eq!(got.read_only, [dir]);
        assert_eq!(got.version.as_deref(), Some("1.4.3"));
        // Its install command fetches the pinned zip and checks its hash.
        let install = m.install_command().join(" ");
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            assert!(
                install.contains("sha256sum -c") && install.contains("1.4.3"),
                "{install}"
            );
        }
    }

    #[test]
    fn every_manifest_parses_with_a_pinned_version_and_known_secrets() {
        let all = manifests();
        assert_eq!(
            all.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["claude-code", "codex", "gemini", "antigravity"]
        );
        let mut profiles: Vec<_> = all.iter().map(|m| m.profile.as_str()).collect();
        profiles.dedup();
        assert_eq!(profiles, ["claude", "codex", "gemini", "antigravity"]);
        for m in all {
            let exact = m.package.version.split('.').count() == 3
                && m.package
                    .version
                    .split('.')
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
            assert!(
                exact,
                "{}: version {:?} must be exact",
                m.id, m.package.version
            );
            // Signed in with keyring secrets, or a login it keeps itself.
            assert!(
                !m.auth.secrets.is_empty() || m.auth.own_login.is_some(),
                "{}",
                m.id
            );
            // From npm under a runtime, or a pinned, checksummed download.
            match (&m.package.npm, &m.package.archive) {
                (Some(_), None) => assert!(m.package.runtime.is_some(), "{}", m.id),
                (None, Some(archive)) => {
                    let sha = archive.linux_x86_64_sha256.as_deref().unwrap_or_default();
                    assert!(
                        sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
                        "{}",
                        m.id
                    );
                }
                _ => panic!("{}: exactly one of npm and archive", m.id),
            }
            for secret in &m.auth.secrets {
                assert!(
                    forge_sandbox::secrets::known(secret).is_some(),
                    "{}: {secret}",
                    m.id
                );
            }
            assert!(!m.quirks.notes.is_empty(), "{}", m.id);
            assert_eq!(by_profile(&m.profile), Some(m));
        }
        assert_eq!(
            manifest("codex").unwrap().install_command().join(" "),
            "npm install -g @agentclientprotocol/codex-acp@2.2.2"
        );
        assert_eq!(
            manifest("gemini").unwrap().default_command(),
            ["gemini", "--acp", "--approval-mode", "default"]
        );
    }

    fn executable(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A fake npm prefix: `bin/<name>` links into `lib/node_modules/<package>`.
    fn npm_install(prefix: &Path, package: &str, bin: &str, version: &str) -> PathBuf {
        let root = prefix.join("lib/node_modules").join(package);
        executable(&root.join("dist/index.js"), "#!/usr/bin/env node\n");
        std::fs::write(
            root.join("package.json"),
            format!(r#"{{"name": "{package}", "version": "{version}"}}"#),
        )
        .unwrap();
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::os::unix::fs::symlink(root.join("dist/index.js"), prefix.join("bin").join(bin))
            .unwrap();
        root
    }

    #[test]
    fn an_npm_adapter_runs_under_node_with_its_package_shown() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().canonicalize().unwrap();
        let root = npm_install(
            &prefix,
            "@agentclientprotocol/codex-acp",
            "codex-acp",
            "2.2.2",
        );
        executable(&prefix.join("bin/node"), "");
        let path = prefix.join("bin");
        let codex = manifest("codex").unwrap();
        let plan = plan(
            Some(codex),
            &codex.default_command(),
            Some(path.as_os_str()),
        )
        .unwrap();
        assert_eq!(
            plan.argv,
            [
                prefix.join("bin/node").display().to_string(),
                root.join("dist/index.js").display().to_string(),
            ]
        );
        assert_eq!(plan.read_only, [prefix.join("bin/node"), root.clone()]);
        assert_eq!(plan.version.as_deref(), Some("2.2.2"));

        // Without node it can't run.
        std::fs::remove_file(prefix.join("bin/node")).unwrap();
        assert_eq!(
            super::plan(
                Some(codex),
                &codex.default_command(),
                Some(path.as_os_str())
            ),
            Err(LaunchError::NoRuntime("codex-acp".into()))
        );
    }

    #[test]
    fn a_program_from_another_package_is_shown_alone_and_must_be_executable() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().canonicalize().unwrap();
        // Same bin name, but another package: no package mount, no version.
        npm_install(&prefix, "codex-acp-impostor", "codex-acp", "9.9.9");
        executable(&prefix.join("bin/node"), "");
        let path = prefix.join("bin");
        let codex = manifest("codex").unwrap();
        let plan = plan(
            Some(codex),
            &codex.default_command(),
            Some(path.as_os_str()),
        )
        .unwrap();
        let script = prefix.join("lib/node_modules/codex-acp-impostor/dist/index.js");
        assert_eq!(plan.argv, [script.display().to_string()]);
        assert_eq!(plan.read_only, [script]);
        assert_eq!(plan.version, None);

        // A profile can't use the agent's command to show a plain file in the sandbox.
        let secret = prefix.join("id_ed25519");
        std::fs::write(&secret, "key").unwrap();
        let command = [secret.display().to_string()];
        assert_eq!(
            super::plan(None, &command, Some(path.as_os_str())),
            Err(LaunchError::NotExecutable(secret.display().to_string()))
        );
        assert_eq!(
            super::plan(
                None,
                &["forge-no-such-agent".into()],
                Some(path.as_os_str())
            ),
            Err(LaunchError::NotInstalled("forge-no-such-agent".into()))
        );
    }

    #[test]
    fn programs_resolve_through_absolute_path_entries_to_their_real_file() {
        let sh = locate("sh", std::env::var_os("PATH").as_deref()).unwrap();
        assert!(sh.is_absolute() && sh.is_file());
        assert_eq!(locate(sh.to_str().unwrap(), None), Some(sh));
        assert_eq!(locate("/", None), None);
        let dir = tempfile::tempdir().unwrap();
        executable(&dir.path().join("x"), "");
        // A relative PATH entry is never searched.
        assert_eq!(locate("x", Some(OsStr::new("."))), None);
    }
}
