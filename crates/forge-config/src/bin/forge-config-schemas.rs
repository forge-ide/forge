//! Writes the config JSON Schemas into the directory given as the only argument.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(dir), None) = (args.next().map(PathBuf::from), args.next()) else {
        eprintln!("usage: forge-config-schemas <dir>");
        return ExitCode::FAILURE;
    };
    for (name, json) in forge_config::schemas() {
        let path = dir.join(name);
        if let Err(err) = std::fs::write(&path, json) {
            eprintln!("forge-config-schemas: writing {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
