//! Tauri 2 shell for the Forge UI. Business logic lives in `forged`, not here.

// Prevents an extra console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("failed to run the Forge desktop app");
}
