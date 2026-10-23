//! Writes the generated TypeScript API types to the path given as the only argument.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(path), None) = (args.next(), args.next()) else {
        eprintln!("usage: forge-api-gen <output.ts>");
        return ExitCode::FAILURE;
    };
    if let Err(err) = std::fs::write(&path, forge_api::typescript::generate()) {
        eprintln!("forge-api-gen: writing {}: {err}", path.to_string_lossy());
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
