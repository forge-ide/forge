//! Daemon startup and shutdown behavior.

mod common;

use std::os::unix::net::UnixListener;

use common::{Daemon, forged_command, temp_home};

#[test]
fn second_daemon_refuses_to_start() {
    let home = temp_home();
    let _first = Daemon::start(home.path());
    let output = forged_command(home.path()).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("already running"), "{stderr}");
}

#[test]
fn stale_socket_is_replaced() {
    let home = temp_home();
    let socket = home.path().join("run/forged.sock");
    // Bind and drop: leaves a socket file nobody listens on, as after a crash.
    drop(UnixListener::bind(&socket).unwrap());
    assert!(socket.exists());
    let daemon = Daemon::start(home.path());
    daemon.stop();
    assert!(!socket.exists(), "clean shutdown removes the socket");
}

#[test]
fn refuses_to_replace_a_non_socket_file() {
    let home = temp_home();
    let socket = home.path().join("run/forged.sock");
    std::fs::write(&socket, "not a socket").unwrap();
    let output = forged_command(home.path()).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read_to_string(&socket).unwrap(), "not a socket");
}

#[test]
fn invalid_user_config_stops_startup() {
    let home = temp_home();
    std::fs::write(
        home.path().join("config/config.toml"),
        "[ui]\nthme = \"dark\"\n",
    )
    .unwrap();
    let output = forged_command(home.path()).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("thme"), "{stderr}");
}

#[test]
fn creates_the_database() {
    let home = temp_home();
    let daemon = Daemon::start(home.path());
    assert!(home.path().join("data/forge.db").exists());
    daemon.stop();
}
