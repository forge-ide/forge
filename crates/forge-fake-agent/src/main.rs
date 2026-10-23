//! `forge-fake-agent <script.yaml>`: an ACP agent on stdio that replays a script.

use std::path::PathBuf;
use std::process::ExitCode;

use agent_client_protocol::ByteStreams;
use forge_fake_agent::Script;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(path), None) = (args.next().map(PathBuf::from), args.next()) else {
        eprintln!("usage: forge-fake-agent <script.yaml>");
        return ExitCode::FAILURE;
    };
    let script = match Script::load(&path) {
        Ok(script) => script,
        Err(e) => {
            eprintln!("forge-fake-agent: {e}");
            return ExitCode::FAILURE;
        }
    };
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
