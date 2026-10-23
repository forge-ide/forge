//! The environment allowlist: what a sandboxed process inherits from the daemon.
//!
//! Everything not listed is dropped, notably `SSH_AUTH_SOCK`, `DBUS_SESSION_BUS_ADDRESS`,
//! `DISPLAY`, `XDG_RUNTIME_DIR`, and every token or API key. Secrets reach agents only through
//! the egress proxy, never through the environment.

/// Variables passed through unchanged, besides every `LC_*`.
pub const PASSTHROUGH_VARS: &[&str] =
    &["PATH", "USER", "LOGNAME", "LANG", "LANGUAGE", "TZ", "TERM"];

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

#[cfg(test)]
mod tests {
    use super::*;

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
