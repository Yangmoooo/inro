use std::io::{Read, Write};
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
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
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
