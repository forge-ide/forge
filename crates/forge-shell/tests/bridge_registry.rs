//! RED 1: Bridge connection registry starts empty and tracks connections
//! by `session_id`. This is the foundation on top of which the Tauri
//! commands dispatch.

use forge_shell::bridge::SessionConnections;

#[tokio::test]
async fn new_registry_is_empty() {
    let connections = SessionConnections::new();
    assert_eq!(connections.len().await, 0);
}
