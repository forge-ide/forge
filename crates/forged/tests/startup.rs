//! Daemon startup and shutdown behavior.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;

use common::{Daemon, expect_exit, temp_home};

#[test]
fn second_daemon_refuses_to_start() {
    let home = temp_home();
    let _first = Daemon::start(home.path());
    let output = expect_exit(home.path());
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
    let output = expect_exit(home.path());
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
    let output = expect_exit(home.path());
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("thme"), "{stderr}");
}

#[test]
fn refuses_shared_directories_without_changing_them() {
    for dir in ["run", "data"] {
        let home = temp_home();
        let path = home.path().join(dir);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let output = expect_exit(home.path());
        assert!(!output.status.success(), "{dir}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("other users"), "{dir}: {stderr}");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "{dir} must not be chmodded");
        assert!(!home.path().join("data/forge.db").exists(), "{dir}");
    }
}

#[test]
fn creates_missing_directories_private() {
    let home = temp_home();
    std::fs::remove_dir(home.path().join("run")).unwrap();
    std::fs::remove_dir(home.path().join("data")).unwrap();
    let daemon = Daemon::start(home.path());
    for dir in ["run", "data"] {
        let mode = std::fs::metadata(home.path().join(dir))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700, "{dir}");
    }
    daemon.stop();
}

#[test]
fn creates_the_database() {
    let home = temp_home();
    let daemon = Daemon::start(home.path());
    assert!(home.path().join("data/forge.db").exists());
    daemon.stop();
}
