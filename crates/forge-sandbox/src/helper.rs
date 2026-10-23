//! The `forge-sandbox-exec` helper: applies a [`Policy`] to its own process, then execs the
//! command, which inherits the restrictions. A separate binary because the workspace forbids
//! `unsafe` code, which rules out `Command::pre_exec`.
//!
//! ```text
//! forge-sandbox-exec light|standard <policy-json> -- <program> [args...]
//! forge-sandbox-exec check-light
//! ```

use std::ffi::OsString;
use std::process::ExitCode;

use crate::policy::Policy;

/// Exit code when the sandbox itself could not be set up.
pub const SETUP_FAILED: u8 = 125;
/// Exit code when the command could not be executed.
pub const EXEC_FAILED: u8 = 127;

/// Entry point for the helper binary.
pub fn main(args: Vec<OsString>) -> ExitCode {
    match run(args) {
        Ok(never) => match never {},
        Err((code, message)) => {
            eprintln!("forge-sandbox-exec: {message}");
            ExitCode::from(code)
        }
    }
}

enum Never {}

fn run(args: Vec<OsString>) -> Result<Never, (u8, String)> {
    let usage = || {
        (
            SETUP_FAILED,
            "usage: forge-sandbox-exec light|standard <policy-json> -- <program> [args...]"
                .to_owned(),
        )
    };
    let mut args = args.into_iter().skip(1);
    let mode = args.next().ok_or_else(usage)?;
    if mode == "check-light" {
        check_light().map_err(|e| (SETUP_FAILED, e))?;
        std::process::exit(0);
    }
    let policy = args.next().ok_or_else(usage)?;
    let policy = Policy::from_json(&policy.to_string_lossy()).map_err(|e| (SETUP_FAILED, e))?;
    if args.next().as_deref() != Some("--".as_ref()) {
        return Err(usage());
    }
    let program = args.next().ok_or_else(usage)?;
    let expected = match mode.to_str() {
        Some("light") => crate::policy::Mode::Light,
        Some("standard") => crate::policy::Mode::Standard,
        _ => return Err(usage()),
    };
    if policy.mode != expected {
        return Err((
            SETUP_FAILED,
            "policy mode does not match the command".into(),
        ));
    }
    enforce(&policy).map_err(|e| (SETUP_FAILED, e))?;
    let err = exec(program.clone(), args.collect());
    Err((
        EXEC_FAILED,
        format!("running {}: {err}", program.to_string_lossy()),
    ))
}

#[cfg(target_os = "linux")]
fn check_light() -> Result<(), String> {
    crate::linux::landlock::restrict(&Policy {
        mode: crate::policy::Mode::Light,
        read: vec![],
        write: vec![],
        connect_ports: vec![],
    })?;
    crate::linux::seccomp::apply(crate::policy::Mode::Light)
}

#[cfg(target_os = "linux")]
fn enforce(policy: &Policy) -> Result<(), String> {
    if policy.mode == crate::policy::Mode::Light {
        crate::linux::landlock::restrict(policy)?;
    }
    crate::linux::seccomp::apply(policy.mode)
}

#[cfg(not(target_os = "linux"))]
fn check_light() -> Result<(), String> {
    Err("Landlock is Linux-only".into())
}

#[cfg(not(target_os = "linux"))]
fn enforce(_: &Policy) -> Result<(), String> {
    Err("the helper only enforces policies on Linux".into())
}

fn exec(program: OsString, args: Vec<OsString>) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    std::process::Command::new(program).args(args).exec()
}
