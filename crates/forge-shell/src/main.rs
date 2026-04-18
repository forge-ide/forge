#[cfg(feature = "webview")]
fn main() {
    forge_shell::window_manager::run().expect("forge-shell failed");
}

#[cfg(not(feature = "webview"))]
fn main() {
    eprintln!("forge-shell requires the `webview` feature to run");
    std::process::exit(1);
}
