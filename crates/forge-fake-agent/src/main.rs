//! `forge-fake-agent <script.yaml>`: an ACP agent on stdio that replays a script.
//! `forge-fake-agent <script.yaml> <args...>`, with a terminal sign-in method's args: that
//! sign-in, which writes the script's login file and exits.

use std::path::PathBuf;
use std::process::ExitCode;

use agent_client_protocol::ByteStreams;
use forge_fake_agent::Script;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next().map(PathBuf::from) else {
        eprintln!("usage: forge-fake-agent <script.yaml> [sign-in args...]");
        return ExitCode::FAILURE;
    };
    let rest: Vec<String> = args.collect();
    let script = match Script::load(&path) {
        Ok(script) => script,
        Err(e) => {
            eprintln!("forge-fake-agent: {e}");
            return ExitCode::FAILURE;
        }
    };
    if !rest.is_empty() {
        let auth = script.auth.as_ref().filter(|a| {
            a.methods
                .iter()
                .any(|m| m.kind == "terminal" && m.args == rest)
        });
        let Some(auth) = auth else {
            eprintln!("forge-fake-agent: no terminal sign-in takes {rest:?}");
            return ExitCode::FAILURE;
        };
        return match forge_fake_agent::write_login(auth) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("forge-fake-agent: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("forge-fake-agent: starting runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let transport = ByteStreams::new(
        tokio::io::stdout().compat_write(),
        tokio::io::stdin().compat(),
    );
    match runtime.block_on(forge_fake_agent::serve(script, transport)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("forge-fake-agent: {e}");
            ExitCode::FAILURE
        }
    }
}
