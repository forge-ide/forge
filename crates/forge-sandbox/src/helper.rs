//! The `forge-sandbox-exec` helper: applies a [`Policy`] to its own process, then execs the
//! command, which inherits the restrictions. A separate binary because the workspace forbids
//! `unsafe` code, which rules out `Command::pre_exec`.
//!
//! ```text
//! forge-sandbox-exec light|standard <policy-json> [--env NAME=value]... -- <program> [args...]
//! forge-sandbox-exec check-light
//! ```
//!
//! `--env` variables are set on the command only, after the restrictions apply, so a loader
//! variable such as `LD_PRELOAD` can't run code in the helper before it restricts itself.

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
            "usage: forge-sandbox-exec light|standard <policy-json> [--env NAME=value]... -- \
             <program> [args...]"
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
    let mut extra: Vec<(String, String)> = Vec::new();
    loop {
        match args.next() {
            Some(a) if a == "--" => break,
            Some(a) if a == "--env" => {
                let pair = args.next().ok_or_else(usage)?;
                let pair = pair.to_str().ok_or_else(usage)?;
                let (name, value) = pair.split_once('=').ok_or_else(usage)?;
                // The launcher filtered these already; this holds even if it didn't.
                if crate::env::is_valid_name(name) && std::env::var_os(name).is_none() {
                    extra.push((name.to_owned(), value.to_owned()));
                }
            }
            _ => return Err(usage()),
        }
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
    if let Some(forward) = &policy.forward {
        // The forwarder has to keep running, so run the command as a child instead of
        // replacing this process with it.
        start_forwarder(forward).map_err(|e| (SETUP_FAILED, e))?;
        let status = std::process::Command::new(&program)
            .args(args)
            .envs(extra)
            .status()
            .map_err(|e| {
                (
                    EXEC_FAILED,
                    format!("running {}: {e}", program.to_string_lossy()),
                )
            })?;
        std::process::exit(exit_code(status));
    }
    let err = exec(program.clone(), args.collect(), extra);
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
        forward: None,
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

/// Listen on `127.0.0.1:<port>` and relay each connection to the proxy's Unix socket, on
/// background threads.
fn start_forwarder(forward: &crate::policy::Forward) -> Result<(), String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", forward.port))
        .map_err(|e| format!("listening on 127.0.0.1:{}: {e}", forward.port))?;
    let socket = forward.socket.clone();
    std::thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let socket = socket.clone();
            std::thread::spawn(move || relay(tcp, &socket));
        }
    });
    Ok(())
}

fn relay(tcp: std::net::TcpStream, socket: &std::path::Path) {
    use std::net::Shutdown;
    let Ok(unix) = std::os::unix::net::UnixStream::connect(socket) else {
        return;
    };
    let (Ok(mut tcp_in), Ok(mut unix_out)) = (tcp.try_clone(), unix.try_clone()) else {
        return;
    };
    let up = std::thread::spawn(move || {
        let _ = std::io::copy(&mut tcp_in, &mut unix_out);
        let _ = unix_out.shutdown(Shutdown::Write);
    });
    let (mut unix_in, mut tcp_out) = (unix, tcp);
    let _ = std::io::copy(&mut unix_in, &mut tcp_out);
    let _ = tcp_out.shutdown(Shutdown::Write);
    let _ = up.join();
}

/// The exit code that reports `status` to whoever waits on this process.
fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

fn exec(program: OsString, args: Vec<OsString>, env: Vec<(String, String)>) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    std::process::Command::new(program)
        .args(args)
        .envs(env)
        .exec()
}

/// The helper arguments that pass `env` to the command it runs.
pub fn env_args(env: &[(String, String)]) -> Vec<String> {
    env.iter()
        .flat_map(|(k, v)| ["--env".to_owned(), format!("{k}={v}")])
        .collect()
}
