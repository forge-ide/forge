//! Tauri 2 shell for the Forge UI. Business logic lives in `forged`, not here.

// Prevents an extra console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod daemon;

fn main() {
    tauri::Builder::default()
        .manage(daemon::Bridge::default())
        .invoke_handler(tauri::generate_handler![
            daemon::daemon_connect,
            daemon::daemon_send,
            daemon::daemon_info,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run the Forge desktop app");
}
