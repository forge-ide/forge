//! Tries one action and reports whether it worked, for the sandbox escape suite. Exits 0 when
//! the action succeeded, 1 when it failed (printing why), and 2 on bad usage.
//!
//! ```text
//! forge-sandbox-probe read <file> | list <dir> | write <file>
//!                     connect-tcp <addr:port> | connect-unix <path> | udp <addr:port>
//!                     signal <pid> | proxy-connect <host:port>
//! ```

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [action, target] = args.as_slice() else {
        eprintln!("usage: forge-sandbox-probe <action> <target>");
        return ExitCode::from(2);
    };
    let result = match action.as_str() {
        "read" => std::fs::read(target).map(drop),
        "list" => std::fs::read_dir(target)
            .and_then(|mut d| d.next().transpose())
            .map(drop),
        "write" => std::fs::write(target, "escaped\n"),
        "connect-tcp" => std::net::TcpStream::connect(target).map(drop),
        "connect-unix" => std::os::unix::net::UnixStream::connect(target).map(drop),
        "udp" => {
            std::net::UdpSocket::bind("0.0.0.0:0").and_then(|s| s.send_to(b"x", target).map(drop))
        }
        "signal" => signal(target),
        "proxy-connect" => proxy_connect(target),
        _ => {
            eprintln!("forge-sandbox-probe: unknown action {action:?}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("forge-sandbox-probe: {action} {target}: {e}");
            ExitCode::from(1)
        }
    }
}

/// Check permission to signal `pid` (signal 0 delivers nothing).
fn signal(pid: &str) -> std::io::Result<()> {
    let pid = pid
        .parse::<i32>()
        .ok()
        .and_then(rustix::process::Pid::from_raw)
        .ok_or_else(|| std::io::Error::other("not a pid"))?;
    rustix::process::test_kill_process(pid).map_err(std::io::Error::from)
}

/// Ask the proxy in `HTTPS_PROXY` to tunnel to `target`; succeeds on a 200 reply.
fn proxy_connect(target: &str) -> std::io::Result<()> {
    use std::io::{Read, Write};
    let url = std::env::var("HTTPS_PROXY")
        .map_err(|_| std::io::Error::other("HTTPS_PROXY is not set"))?;
    let addr = url
        .strip_prefix("http://")
        .unwrap_or(&url)
        .trim_end_matches('/');
    let mut stream = std::net::TcpStream::connect(addr)?;
    write!(
        stream,
        "CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n"
    )?;
    let mut reply = [0u8; 256];
    let n = stream.read(&mut reply)?;
    let reply = String::from_utf8_lossy(&reply[..n]);
    let status = reply.lines().next().unwrap_or_default().to_owned();
    if status.starts_with("HTTP/1.1 200") {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("proxy refused: {status}")))
    }
}
