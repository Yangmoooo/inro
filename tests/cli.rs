use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output};
use std::time::Duration;
use std::{fs, thread};

fn run_inro(home: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_inro"))
        .args(args)
        .env("INRO_HOME", home)
        .env_remove("INRO_UPSTREAMS")
        .output()
        .expect("failed to run inro")
}

fn serve_once(body: &'static str) -> (String, thread::JoinHandle<()>) {
    serve_once_with_timeout(body, Duration::from_secs(5))
}

fn serve_once_with_timeout(
    body: &'static str,
    timeout: Duration,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut request = [0u8; 2048];
                    let _ = stream.read(&mut request);
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .unwrap();
                    break;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        }
    });
    (format!("http://{address}/registry.toml"), handle)
}

#[test]
fn source_update_rejects_invalid_registry() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("inro");
    fs::create_dir_all(&home).unwrap();
    let (url, server) = serve_once("[tool]\nver = \"v1.0.0\"\n");
    fs::write(
        home.join("config.toml"),
        format!(
            "upstreams = [{{ name = \"test\", priority = 0, enabled = true, url = \"{url}\" }}]\n"
        ),
    )
    .unwrap();

    let output = run_inro(&home, &["source", "update"]);
    server.join().unwrap();

    assert!(
        !output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!home.join("registry/00-test.toml").exists());
}

#[test]
fn source_update_installs_valid_registry() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("inro");
    fs::create_dir_all(&home).unwrap();
    let (url, server) = serve_once("[tool.remote.github]\nrepo = \"test/tool\"\n");
    fs::write(
        home.join("config.toml"),
        format!(
            "upstreams = [{{ name = \"test\", priority = 0, enabled = true, url = \"{url}\" }}]\n"
        ),
    )
    .unwrap();

    let output = run_inro(&home, &["source", "update"]);
    server.join().unwrap();

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(home.join("registry/00-test.toml").exists());
}

#[test]
fn doctor_exits_nonzero_when_errors_are_found() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("inro");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("manifest.json"), "{ invalid json").unwrap();

    let output = run_inro(&home, &["doctor"]);

    assert!(
        !output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("error(s)"));
}

#[test]
fn double_verbose_adds_debug_tracing() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("inro");

    let verbose = run_inro(&home, &["-v", "env"]);
    let debug = run_inro(&home, &["-vv", "env"]);

    assert!(verbose.status.success());
    assert!(debug.status.success());
    let verbose_stderr = String::from_utf8_lossy(&verbose.stderr);
    let debug_stderr = String::from_utf8_lossy(&debug.stderr);
    assert!(!verbose_stderr.contains("Resolved INRO_HOME"));
    assert!(debug_stderr.contains("Resolved INRO_HOME"), "stderr: {debug_stderr}");
    assert!(debug_stderr.contains("Loaded config"), "stderr: {debug_stderr}");
}

#[test]
fn test_server_stops_waiting_without_a_connection() {
    let started = std::time::Instant::now();
    let (_url, server) = serve_once_with_timeout("unused", Duration::from_millis(50));

    server.join().unwrap();

    assert!(started.elapsed() < Duration::from_secs(1));
}
