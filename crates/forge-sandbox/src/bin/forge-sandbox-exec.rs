//! Applies a sandbox policy to itself, then execs the command. See `forge_sandbox::helper`.

fn main() -> std::process::ExitCode {
    forge_sandbox::helper::main(std::env::args_os().collect())
}
