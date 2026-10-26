//! The environment allowlist: what a sandboxed process inherits from the daemon.
//!
//! Everything not listed is dropped, notably `SSH_AUTH_SOCK`, `DBUS_SESSION_BUS_ADDRESS`,
//! `DISPLAY`, `XDG_RUNTIME_DIR`, and every token or API key. Secrets reach agents only through
//! the egress proxy, never through the environment.

/// Variables passed through unchanged, besides every `LC_*`.
pub const PASSTHROUGH_VARS: &[&str] =
    &["PATH", "USER", "LOGNAME", "LANG", "LANGUAGE", "TZ", "TERM"];

/// Shell startup files a sandboxed process doesn't read, set in every sandbox: zsh reads
/// `$ZDOTDIR/.zshenv` (by default in the writable home) before every `-c` script, bash and sh
/// read the file `$BASH_ENV` and `$ENV` name, and bash reads `~/.bashrc` for `-c` when its
/// stdin is a socket (as Node gives its children) unless `SHLVL` says it is nested. Any of
/// them would run code the agent wrote ahead of a command Forge decided. `/dev/null` is no
/// directory, so nothing can be put there.
pub const STARTUP_FILES_OFF: &[(&str, &str)] = &[
    ("ZDOTDIR", "/dev/null"),
    ("BASH_ENV", ""),
    ("ENV", ""),
    ("SHLVL", "1"),
];

/// [`STARTUP_FILES_OFF`], as variables.
pub(crate) fn startup_files_off() -> impl Iterator<Item = (String, String)> {
    STARTUP_FILES_OFF
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
}

/// Whether the daemon's `name` is passed into sandboxed processes.
pub fn is_passed_through(name: &str) -> bool {
    PASSTHROUGH_VARS.contains(&name) || name.starts_with("LC_")
}

/// The allowlisted subset of `vars`, in order.
pub(crate) fn passthrough(vars: impl Iterator<Item = (String, String)>) -> Vec<(String, String)> {
    vars.filter(|(name, _)| is_passed_through(name)).collect()
}

/// The daemon's own allowlisted variables.
pub(crate) fn from_daemon() -> Vec<(String, String)> {
    passthrough(std::env::vars())
}

/// Whether `name` can be an environment variable's name: not empty, and no `=` or NUL, either
/// of which would make the entry mean something else to the process (`PATH=x=` is read as a
/// second `PATH` by shells).
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['=', '\0'])
}

/// The variables from a command's own `env` (which an agent chooses for its terminals) that
/// may reach it: valid names whose value has no NUL, and none the sandbox sets itself, so the
/// sandbox's `PATH`, `HOME`, `TMPDIR`, and proxy variables always stand.
pub(crate) fn extra(
    sandbox: &[(String, String)],
    env: &[(String, String)],
) -> Vec<(String, String)> {
    env.iter()
        .filter(|(k, v)| {
            is_valid_name(k) && !v.contains('\0') && !sandbox.iter().any(|(s, _)| s == k)
        })
        .cloned()
        .collect()
}

/// The value of `name` in `env`.
pub(crate) fn lookup(env: &[(String, String)], name: &str) -> Option<String> {
    env.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_variables_never_replace_the_sandboxs_or_smuggle_names() {
        let pair = |k: &str, v: &str| (k.to_owned(), v.to_owned());
        let sandbox = vec![pair("PATH", "/usr/bin"), pair("HOME", "/s/home")];
        let env = vec![
            pair("PATH", "/agent"),
            pair("PATH=/agent:/usr/bin", ""),
            pair("", "x"),
            pair("NUL\0", "x"),
            pair("BAD_VALUE", "a\0b"),
            pair("RUST_LOG", "debug"),
        ];
        assert_eq!(extra(&sandbox, &env), [pair("RUST_LOG", "debug")]);
    }

    #[test]
    fn only_allowlisted_variables_pass() {
        let vars = [
            ("PATH", "/usr/bin"),
            ("LC_ALL", "C"),
            ("HOME", "/home/u"),
            ("SSH_AUTH_SOCK", "/run/user/1000/ssh"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
            ("ANTHROPIC_API_KEY", "sk-secret"),
            ("GITHUB_TOKEN", "ghp_x"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("TERM", "xterm"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()));
        let names: Vec<String> = passthrough(vars).into_iter().map(|(k, _)| k).collect();
        assert_eq!(names, ["PATH", "LC_ALL", "TERM"]);
    }
}
