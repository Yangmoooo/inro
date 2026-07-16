use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;
use std::{fs, thread};

fn run_inro(home: &std::path::Path, args: &[&str]) -> Output { run_inro_with_env(home, args, &[]) }

fn run_inro_with_env(home: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_inro"));
    command
        .args(args)
        .env("INRO_HOME", home)
        .env_remove("INRO_UPSTREAMS")
        .env_remove("INRO_GITHUB_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env("no_proxy", "127.0.0.1,localhost");
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("failed to run inro")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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

fn closed_local_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{address}")
}

#[cfg(unix)]
fn setup_tool_home(root: &Path) -> (PathBuf, PathBuf) {
    let home = root.join("inro");
    let bin_dir = root.join("bin");
    fs::create_dir_all(home.join("registry.d")).unwrap();
    fs::write(home.join("config.toml"), format!("bin_dir = {:?}\nupstreams = []\n", bin_dir))
        .unwrap();
    fs::write(home.join("registry.d/tool.toml"), "[tool.remote.github]\nrepo = \"test/tool\"\n")
        .unwrap();
    (home, bin_dir)
}

#[cfg(unix)]
fn tool_binary() -> Vec<u8> { b"\x7fELFfixture".to_vec() }

#[cfg(unix)]
fn serve_tool_release(
    version: &str,
    asset: Vec<u8>,
    expect_download: bool,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let base_url = format!("http://{address}");
    let asset_name = "tool";
    let release = serde_json::json!([{
        "tag_name": version,
        "html_url": format!("{base_url}/releases/{version}"),
        "prerelease": false,
        "draft": false,
        "created_at": "2026-01-01T00:00:00Z",
        "published_at": "2026-01-01T00:00:00Z",
        "assets": [{
            "name": asset_name,
            "size": asset.len(),
            "browser_download_url": format!("{base_url}/assets/{asset_name}")
        }]
    }]);
    let release = serde_json::to_vec(&release).unwrap();

    let handle = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let expected_requests = if expect_download { 2 } else { 1 };
        let mut served = 0;
        while served < expected_requests && std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                    let mut request = Vec::new();
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        let mut chunk = [0u8; 1024];
                        let read = stream.read(&mut chunk).unwrap();
                        assert!(read > 0, "connection closed before request headers completed");
                        request.extend_from_slice(&chunk[..read]);
                        assert!(request.len() <= 16 * 1024, "request headers too large");
                    }
                    let request = String::from_utf8_lossy(&request);
                    let target = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .and_then(|target| target.split('?').next())
                        .unwrap_or_default();
                    let (content_type, body) = match target {
                        "/repos/test/tool/releases" => ("application/json", release.as_slice()),
                        "/assets/tool" => ("application/octet-stream", asset.as_slice()),
                        _ => panic!("unexpected request target: {target}"),
                    };
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .unwrap();
                    stream.write_all(body).unwrap();
                    stream.flush().unwrap();
                    served += 1;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        }
        assert_eq!(served, expected_requests, "unexpected fixture request count");
    });

    (base_url, handle)
}

#[cfg(unix)]
fn install_tool(home: &Path, version: &str) {
    let (api_url, server) = serve_tool_release(version, tool_binary(), true);
    let output =
        run_inro_with_env(home, &["-v", "install", "tool"], &[("INRO_GITHUB_API_URL", &api_url)]);
    server.join().unwrap();
    assert_success(&output);
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

#[cfg(unix)]
#[test]
fn install_commits_files_link_receipt_and_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let (home, bin_dir) = setup_tool_home(temp.path());
    let (api_url, server) = serve_tool_release("v1.0.0", tool_binary(), true);

    let output =
        run_inro_with_env(&home, &["-v", "install", "tool"], &[("INRO_GITHUB_API_URL", &api_url)]);
    server.join().unwrap();

    assert_success(&output);
    let install_dir = home.join("pkgs/tool/v1.0.0");
    assert_eq!(fs::read(install_dir.join("tool")).unwrap(), b"\x7fELFfixture");
    assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), install_dir.join("tool"));

    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(install_dir.join("inro-receipt.json")).unwrap()).unwrap();
    assert_eq!(receipt["name"], "tool");
    assert_eq!(receipt["version"], "v1.0.0");

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["packages"]["tool"]["current_version"], "v1.0.0");
    assert_eq!(manifest["packages"]["tool"]["versions"]["v1.0.0"], receipt);
}

#[cfg(unix)]
#[test]
fn install_commits_successful_packages_when_batch_partially_fails() {
    let temp = tempfile::tempdir().unwrap();
    let (home, bin_dir) = setup_tool_home(temp.path());
    let (api_url, server) = serve_tool_release("v1.0.0", tool_binary(), true);

    let output = run_inro_with_env(
        &home,
        &["install", "tool", "missing"],
        &[("INRO_GITHUB_API_URL", &api_url)],
    );
    server.join().unwrap();

    assert!(
        !output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(home.join("pkgs/tool/v1.0.0/inro-receipt.json").exists());
    assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), home.join("pkgs/tool/v1.0.0/tool"));
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["packages"]["tool"]["current_version"], "v1.0.0");
    assert!(manifest["packages"].get("missing").is_none());
}

#[cfg(unix)]
#[test]
fn update_retains_old_version_and_activates_new_version() {
    let temp = tempfile::tempdir().unwrap();
    let (home, bin_dir) = setup_tool_home(temp.path());
    install_tool(&home, "v1.0.0");

    let (v2_api_url, v2_server) = serve_tool_release("v2.0.0", tool_binary(), true);
    let update =
        run_inro_with_env(&home, &["update", "tool"], &[("INRO_GITHUB_API_URL", &v2_api_url)]);
    v2_server.join().unwrap();

    assert_success(&update);
    let v1_dir = home.join("pkgs/tool/v1.0.0");
    let v2_dir = home.join("pkgs/tool/v2.0.0");
    assert!(v1_dir.join("inro-receipt.json").exists());
    assert!(v2_dir.join("inro-receipt.json").exists());
    assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), v2_dir.join("tool"));

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("manifest.json")).unwrap()).unwrap();
    let state = &manifest["packages"]["tool"];
    assert_eq!(state["current_version"], "v2.0.0");
    assert!(state["versions"].get("v1.0.0").is_some());
    assert!(state["versions"].get("v2.0.0").is_some());
}

#[cfg(unix)]
#[test]
fn update_does_not_reinstall_an_up_to_date_package() {
    let temp = tempfile::tempdir().unwrap();
    let (home, bin_dir) = setup_tool_home(temp.path());
    install_tool(&home, "v1.0.0");
    let manifest_before = fs::read(home.join("manifest.json")).unwrap();

    let (update_api_url, update_server) = serve_tool_release("v1.0.0", tool_binary(), false);
    let update =
        run_inro_with_env(&home, &["update", "tool"], &[("INRO_GITHUB_API_URL", &update_api_url)]);
    update_server.join().unwrap();

    assert_success(&update);
    assert!(String::from_utf8_lossy(&update.stderr).contains("up to date"));
    assert_eq!(fs::read(home.join("manifest.json")).unwrap(), manifest_before);
    assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), home.join("pkgs/tool/v1.0.0/tool"));
}

#[cfg(unix)]
#[test]
fn update_skips_pinned_package_unless_forced_and_preserves_pin() {
    let temp = tempfile::tempdir().unwrap();
    let (home, bin_dir) = setup_tool_home(temp.path());
    install_tool(&home, "v1.0.0");
    let pin = run_inro(&home, &["pin", "tool"]);
    assert!(pin.status.success());
    let manifest_before = fs::read(home.join("manifest.json")).unwrap();

    let unavailable_api = closed_local_url();
    let skipped = run_inro_with_env(
        &home,
        &["-v", "update", "tool"],
        &[("INRO_GITHUB_API_URL", &unavailable_api)],
    );
    assert!(
        skipped.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&skipped.stdout),
        String::from_utf8_lossy(&skipped.stderr)
    );
    assert!(String::from_utf8_lossy(&skipped.stderr).contains("pinned, skipping"));
    assert_eq!(fs::read(home.join("manifest.json")).unwrap(), manifest_before);

    let (update_api_url, update_server) = serve_tool_release("v2.0.0", tool_binary(), true);
    let forced = run_inro_with_env(
        &home,
        &["update", "--force", "tool"],
        &[("INRO_GITHUB_API_URL", &update_api_url)],
    );
    update_server.join().unwrap();

    assert_success(&forced);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("manifest.json")).unwrap()).unwrap();
    let state = &manifest["packages"]["tool"];
    assert_eq!(state["current_version"], "v2.0.0");
    assert_eq!(state["pinned"], true);
    assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), home.join("pkgs/tool/v2.0.0/tool"));
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
fn update_skips_unlinked_package() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("inro");
    fs::create_dir_all(home.join("registry.d")).unwrap();
    fs::write(home.join("config.toml"), "upstreams = []\n").unwrap();
    fs::write(home.join("registry.d/tool.toml"), "[tool.remote.github]\nrepo = \"test/tool\"\n")
        .unwrap();
    let manifest = serde_json::json!({
        "schema_version": 2,
        "packages": {
            "tool": {
                "current_version": null,
                "versions": {
                    "v1.0.0": {
                        "name": "tool",
                        "version": "v1.0.0",
                        "remote": { "github": { "repo": "test/tool", "asset": {} } },
                        "installed_at": "2026-01-01T00:00:00Z",
                        "install_subdir": "tool/v1.0.0",
                        "binaries": [{ "name": "tool", "bin_subpath": "tool" }]
                    }
                },
                "pinned": false
            }
        }
    });
    fs::write(home.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    let manifest_before = fs::read(home.join("manifest.json")).unwrap();
    let unavailable_api = closed_local_url();

    let output =
        run_inro_with_env(&home, &["-v", "update"], &[("INRO_GITHUB_API_URL", &unavailable_api)]);

    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("unlinked, skipping"));
    let manifest_after = fs::read(home.join("manifest.json")).unwrap();
    assert_eq!(manifest_after, manifest_before);
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_after).unwrap();
    assert!(manifest["packages"]["tool"]["current_version"].is_null());
}

#[test]
fn test_server_stops_waiting_without_a_connection() {
    let started = std::time::Instant::now();
    let (_url, server) = serve_once_with_timeout("unused", Duration::from_millis(50));

    server.join().unwrap();

    assert!(started.elapsed() < Duration::from_secs(1));
}
