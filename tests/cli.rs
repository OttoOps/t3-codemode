use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use t3_codemode::platform::SERVICE;

struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn t3cm(home: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_t3cm"));
    cmd.env("T3CODE_HOME", home)
        .env("T3CM_T3_BIN", home.join("no-t3"))
        .env("T3CM_CONFIG", home.join("none.json"))
        .env("RUST_LOG", "info");
    cmd
}

#[test]
fn serve_appends_to_the_log_file_and_rolls_a_big_one() {
    let home = tempfile::tempdir().unwrap();
    let log = home.path().join("logs").join("t3cm.log");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    std::fs::write(&log, vec![b'x'; (10 << 20) + 1]).unwrap();
    let child = t3cm(home.path())
        .args(["serve", "--port", "0", "--no-watcher", "--state-dir"])
        .arg(home.path().join("state"))
        .arg("--log-file")
        .arg(&log)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _guard = Kill(child);
    let deadline = Instant::now() + Duration::from_secs(30);
    let text = loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        if text.contains("serving MCP") || Instant::now() > deadline {
            break text;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(text.contains("serving MCP"), "log: {text}");
    assert!(!text.contains('\u{1b}'), "no ANSI colour in a log file: {text}");
    let rolled = home.path().join("logs").join("t3cm.log.1");
    assert_eq!(std::fs::metadata(rolled).unwrap().len(), (10 << 20) + 1);
}

#[test]
fn status_names_the_start_command_and_the_logs_when_nothing_serves() {
    let home = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let out = t3cm(home.path()).args(["status", "--port", &port.to_string()]).output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(SERVICE.start), "{err}");
    assert!(err.contains(SERVICE.logs), "{err}");
}

#[test]
fn serve_writes_a_fatal_error_to_the_log_file() {
    let home = tempfile::tempdir().unwrap();
    let log = home.path().join("t3cm.log");
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port().to_string();
    let out = t3cm(home.path())
        .args(["serve", "--no-watcher", "--port", &port, "--state-dir"])
        .arg(home.path().join("state"))
        .arg("--log-file")
        .arg(&log)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.contains(&format!("binding 127.0.0.1:{port}")), "log: {text}");
}

#[cfg(unix)]
#[test]
fn sigterm_stops_serve_promptly_while_a_board_streams_events() {
    use std::io::{BufRead, BufReader, Write};
    let home = tempfile::tempdir().unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let child = t3cm(home.path())
        .args(["serve", "--no-watcher", "--port", &port.to_string(), "--state-dir"])
        .arg(home.path().join("state"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut guard = Kill(child);
    let deadline = Instant::now() + Duration::from_secs(30);
    let stream = loop {
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => break s,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => panic!("serve never listened: {e}"),
        }
    };
    let mut w = stream.try_clone().unwrap();
    write!(w, "GET /api/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/event-stream\r\n\r\n").unwrap();
    let first_by = Instant::now() + Duration::from_secs(15);
    let mut lines = BufReader::new(stream);
    let mut line = String::new();
    while !line.starts_with("event: state") {
        let left = first_by.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "no state event within 15s");
        lines.get_ref().set_read_timeout(Some(left)).unwrap();
        line.clear();
        assert!(lines.read_line(&mut line).expect("no state event within 15s") > 0, "the stream closed early");
    }
    unsafe { libc::kill(guard.0.id() as i32, libc::SIGTERM) };
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(s) = guard.0.try_wait().unwrap() {
            break Some(s);
        }
        if Instant::now() > deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let status = status.expect("serve still runs 5s after SIGTERM with a board stream open");
    assert!(status.success(), "serve exited with {status} after SIGTERM");
}
