use std::io::{ErrorKind, Read, Write};
use std::net::TcpListener;
#[cfg(unix)]
use std::net::TcpStream;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Stdio;
use std::process::{Command, Output};
use std::time::Duration;
use std::{fs, thread};

fn run_inro(home: &std::path::Path, args: &[&str]) -> Output { run_inro_with_env(home, args, &[]) }

fn run_inro_with_env(home: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    inro_command_with_env(home, args, envs).output().expect("failed to run inro")
}

fn inro_command_with_env(home: &Path, args: &[&str], envs: &[(&str, &str)]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_inro"));
    command
        .args(args)
        .env("INRO_HOME", home)
        .env_remove("INRO_UPSTREAMS")
        .env_remove("INRO_TEST_GITHUB_API_URL")
        .env_remove("INRO_GITHUB_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("VISUAL")
        .env_remove("EDITOR")
        .env("NO_PROXY", "127.0.0.1,localhost")
        .env("no_proxy", "127.0.0.1,localhost");
    for (key, value) in envs {
        command.env(key, value);
    }
    command
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn manifest_receipt(name: &str, version: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "version": version,
        "remote": { "github": { "repo": format!("test/{name}"), "asset": {} } },
        "installed_at": "2026-01-01T00:00:00Z",
        "install_subdir": format!("{name}/{version}"),
        "binaries": [{ "name": name, "bin_subpath": name }]
    })
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
fn read_request_headers(stream: &mut TcpStream) -> String {
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

    let mut request = Vec::new();
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let mut chunk = [0u8; 1024];
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "connection closed before request headers completed");
        request.extend_from_slice(&chunk[..read]);
        assert!(request.len() <= 16 * 1024, "request headers too large");
    }

    String::from_utf8(request).unwrap()
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
fn setup_direct_tool_home(root: &Path, versions: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let home = root.join("inro");
    let bin_dir = root.join("bin");
    fs::create_dir_all(home.join("registry.d")).unwrap();
    fs::write(home.join("config.toml"), format!("bin_dir = {:?}\nupstreams = []\n", bin_dir))
        .unwrap();
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let mut registry = String::from("[tool]\n");
    for (version, url) in versions {
        registry
            .push_str(&format!("[tool.remote.direct.\"{version}\"]\n\"{platform}\" = \"{url}\"\n"));
    }
    fs::write(home.join("registry.d/tool.toml"), registry).unwrap();
    (home, bin_dir)
}

#[cfg(unix)]
fn tool_binary() -> Vec<u8> { b"\x7fELFfixture".to_vec() }

#[cfg(unix)]
fn serve_direct_asset(asset: Vec<u8>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_request_headers(&mut stream);
                    assert!(request.starts_with("GET /downloads/tool?source=test HTTP/1.1"));
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        asset.len()
                    )
                    .unwrap();
                    stream.write_all(&asset).unwrap();
                    stream.flush().unwrap();
                    return;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "direct download was not requested"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        }
    });
    (format!("http://{address}/downloads/tool?source=test"), handle)
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum ReleaseEndpoint {
    Latest,
    List,
    Tag,
    LatestThenList,
    LatestThenSecondPage,
}

#[cfg(unix)]
fn serve_tool_release(
    version: &str,
    asset: Vec<u8>,
    expect_download: bool,
) -> (String, thread::JoinHandle<()>) {
    serve_tool_release_at(ReleaseEndpoint::Latest, version, asset, expect_download)
}

#[cfg(unix)]
fn serve_tool_tag_release(
    version: &str,
    asset: Vec<u8>,
    expect_download: bool,
) -> (String, thread::JoinHandle<()>) {
    serve_tool_release_at(ReleaseEndpoint::Tag, version, asset, expect_download)
}

#[cfg(unix)]
fn serve_tool_release_list(version: &str) -> (String, thread::JoinHandle<()>) {
    serve_tool_release_at(ReleaseEndpoint::List, version, tool_binary(), false)
}

#[cfg(unix)]
fn serve_tool_release_with_fallback(
    version: &str,
    asset: Vec<u8>,
) -> (String, thread::JoinHandle<()>) {
    serve_tool_release_at(ReleaseEndpoint::LatestThenList, version, asset, true)
}

#[cfg(unix)]
fn serve_tool_release_on_second_page(
    version: &str,
    asset: Vec<u8>,
) -> (String, thread::JoinHandle<()>) {
    serve_tool_release_at(ReleaseEndpoint::LatestThenSecondPage, version, asset, true)
}

#[cfg(unix)]
fn serve_tool_release_at(
    endpoint: ReleaseEndpoint,
    version: &str,
    asset: Vec<u8>,
    expect_download: bool,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let base_url = format!("http://{address}");
    let asset_name = "tool";
    let release = serde_json::json!({
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
    });
    let release_page = (0..30)
        .map(|index| {
            let mut page_release = release.clone();
            if index > 0 {
                page_release["tag_name"] = serde_json::json!(format!("v0.0.{index}"));
            }
            page_release
        })
        .collect::<Vec<_>>();
    let release_list = serde_json::to_vec(&release_page).unwrap();
    let release = serde_json::to_vec(&release).unwrap();
    let empty_latest = serde_json::json!({
        "tag_name": "v3.0.0",
        "html_url": format!("{base_url}/releases/v3.0.0"),
        "prerelease": false,
        "draft": false,
        "created_at": "2026-02-01T00:00:00Z",
        "published_at": "2026-02-01T00:00:00Z",
        "assets": []
    });
    let unavailable_release_page = serde_json::to_vec(&vec![empty_latest.clone(); 30]).unwrap();
    let empty_latest = serde_json::to_vec(&empty_latest).unwrap();
    let tag_target = format!("/repos/test/tool/releases/tags/{version}");

    let handle = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let release_requests = match endpoint {
            ReleaseEndpoint::LatestThenList => 2,
            ReleaseEndpoint::LatestThenSecondPage => 3,
            _ => 1,
        };
        let expected_requests = release_requests + usize::from(expect_download);
        let mut served = 0;
        while served < expected_requests && std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_request_headers(&mut stream);
                    let request_target = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or_default();
                    let target = request_target.split('?').next().unwrap_or_default();
                    let (content_type, body) = if target == "/repos/test/tool/releases/latest"
                        && matches!(endpoint, ReleaseEndpoint::Latest)
                    {
                        ("application/json", release.as_slice())
                    } else if target == "/repos/test/tool/releases/latest"
                        && matches!(
                            endpoint,
                            ReleaseEndpoint::LatestThenList | ReleaseEndpoint::LatestThenSecondPage
                        )
                    {
                        ("application/json", empty_latest.as_slice())
                    } else if target == "/repos/test/tool/releases"
                        && matches!(
                            endpoint,
                            ReleaseEndpoint::List
                                | ReleaseEndpoint::LatestThenList
                                | ReleaseEndpoint::LatestThenSecondPage
                        )
                    {
                        assert!(
                            request_target.contains("per_page=30"),
                            "request: {request_target}"
                        );
                        if matches!(endpoint, ReleaseEndpoint::LatestThenSecondPage)
                            && request_target.contains("page=1")
                        {
                            ("application/json", unavailable_release_page.as_slice())
                        } else {
                            let expected_page =
                                if matches!(endpoint, ReleaseEndpoint::LatestThenSecondPage) {
                                    2
                                } else {
                                    1
                                };
                            assert!(
                                request_target.contains(&format!("page={expected_page}")),
                                "request: {request_target}"
                            );
                            ("application/json", release_list.as_slice())
                        }
                    } else if target == tag_target.as_str()
                        && matches!(endpoint, ReleaseEndpoint::Tag)
                    {
                        ("application/json", release.as_slice())
                    } else if target == "/assets/tool" {
                        ("application/octet-stream", asset.as_slice())
                    } else {
                        panic!("unexpected request target: {target}")
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
    let output = run_inro_with_env(
        home,
        &["-v", "install", "tool"],
        &[("INRO_TEST_GITHUB_API_URL", &api_url)],
    );
    server.join().unwrap();
    assert_success(&output);
}

#[cfg(unix)]
fn setup_learned_tool_home(root: &Path) -> (PathBuf, PathBuf) {
    let (home, bin_dir) = setup_tool_home(root);
    install_tool(&home, "v1.0.0");
    fs::write(home.join("config.toml"), format!("bin_dir = {bin_dir:?}\n")).unwrap();
    fs::remove_file(home.join("registry.d/tool.toml")).unwrap();
    fs::create_dir_all(home.join("registry")).unwrap();
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    fs::write(
        home.join("registry/00-default.toml"),
        format!("[tool.remote.github]\nrepo = \"test/tool\"\n[tool.remote.github.asset]\n{platform} = \"tool\"\n"),
    )
    .unwrap();
    let auto_path = home.join("registry/auto.toml");
    fs::write(
        &auto_path,
        format!("[tool.remote.github.asset]\n{platform} = \"old-name.zip\"\nother-platform = \"keep.zip\"\n"),
    )
    .unwrap();
    (home, bin_dir)
}

#[cfg(unix)]
#[test]
fn update_recovers_from_stale_auto_asset_using_upstream_rule() {
    let temp = tempfile::tempdir().unwrap();
    let (home, _) = setup_learned_tool_home(temp.path());
    let auto_path = home.join("registry/auto.toml");
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let (api_url, server) = serve_tool_release("v2.0.0", tool_binary(), true);
    let output =
        run_inro_with_env(&home, &["update", "tool"], &[("INRO_TEST_GITHUB_API_URL", &api_url)]);
    assert_success(&output);
    server.join().unwrap();
    let auto: toml::Value = toml::from_str(&fs::read_to_string(auto_path).unwrap()).unwrap();
    assert!(auto["tool"]["remote"]["github"]["asset"].get(&platform).is_none());
    assert_eq!(
        auto["tool"]["remote"]["github"]["asset"]["other-platform"].as_str(),
        Some("keep.zip")
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("no longer matches"));
}

#[cfg(unix)]
#[test]
fn failed_auto_recovery_preserves_selection_and_installed_version() {
    let temp = tempfile::tempdir().unwrap();
    let (home, bin_dir) = setup_learned_tool_home(temp.path());
    let auto_path = home.join("registry/auto.toml");
    let auto_before = fs::read(&auto_path).unwrap();
    let manifest_before = fs::read(home.join("manifest.json")).unwrap();
    let link_before = fs::read_link(bin_dir.join("tool")).unwrap();
    let (api_url, server) = serve_tool_release("v2.0.0", b"invalid binary".to_vec(), true);
    let output = run_inro_with_env(
        &home,
        &["-v", "update", "tool"],
        &[("INRO_TEST_GITHUB_API_URL", &api_url)],
    );
    server.join().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no longer matches"));
    assert_eq!(fs::read(&auto_path).unwrap(), auto_before);
    assert_eq!(fs::read(home.join("manifest.json")).unwrap(), manifest_before);
    assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), link_before);

    let api_url = closed_local_url();
    let output = run_inro_with_env(
        &home,
        &["-v", "update", "tool"],
        &[("INRO_TEST_GITHUB_API_URL", &api_url)],
    );
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("no longer matches"));
    assert_eq!(fs::read(auto_path).unwrap(), auto_before);
}

#[cfg(unix)]
#[test]
fn user_asset_rule_is_strict_even_when_identical_to_auto() {
    let temp = tempfile::tempdir().unwrap();
    let (home, _) = setup_learned_tool_home(temp.path());
    let auto_path = home.join("registry/auto.toml");
    let auto_before = fs::read(&auto_path).unwrap();
    fs::write(home.join("registry.d/local.toml"), &auto_before).unwrap();
    let (api_url, server) = serve_tool_release("v2.0.0", tool_binary(), false);
    let output = run_inro_with_env(
        &home,
        &["-v", "update", "tool"],
        &[("INRO_TEST_GITHUB_API_URL", &api_url)],
    );
    server.join().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("old-name.zip"), "{stderr}");
    assert!(!stderr.contains("no longer matches"), "{stderr}");
    assert_eq!(fs::read(auto_path).unwrap(), auto_before);
}

#[cfg(unix)]
#[test]
fn stale_auto_without_a_verified_platform_reports_reselection_hint_without_verbose() {
    let temp = tempfile::tempdir().unwrap();
    let (home, _) = setup_learned_tool_home(temp.path());
    fs::write(
        home.join("registry/00-default.toml"),
        "[tool.remote.github]\nrepo = \"test/tool\"\n",
    )
    .unwrap();
    let auto_path = home.join("registry/auto.toml");
    let before = fs::read(&auto_path).unwrap();
    let (api_url, server) = serve_tool_release("v2.0.0", tool_binary(), false);
    let output =
        run_inro_with_env(&home, &["update", "tool"], &[("INRO_TEST_GITHUB_API_URL", &api_url)]);
    server.join().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("interactive terminal"), "{stderr}");
    assert!(stderr.contains("inro clean --assets tool"), "{stderr}");
    assert_eq!(fs::read(auto_path).unwrap(), before);
}

#[test]
fn clean_assets_supports_package_all_and_dry_run_without_touching_installed_files() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    fs::create_dir_all(home.join("registry")).unwrap();
    fs::create_dir_all(home.join("registry.d")).unwrap();
    fs::create_dir_all(home.join("pkgs/tool/v1")).unwrap();
    fs::write(home.join("pkgs/tool/v1/tool"), "keep binary").unwrap();
    fs::write(home.join("manifest.json"), "invalid manifest, unused for asset cleanup").unwrap();
    let user = "[tool.remote.github.asset]\nlinux-x86_64 = \"user.zip\"\n";
    fs::write(home.join("registry.d/local.toml"), user).unwrap();
    fs::write(home.join("registry/00-default.toml"), "upstream fixture").unwrap();
    let auto_path = home.join("registry/auto.toml");
    let original = "# remembered choices\n[tool.remote.github.asset]\nlinux-x86_64 = \"old.zip\"\nmacos-aarch64 = \"old-mac.zip\"\n[other.remote.github.asset]\nlinux-x86_64 = \"other.zip\"\n";
    fs::write(&auto_path, original).unwrap();

    let dry_run = run_inro(home, &["clean", "--assets", "tool", "--dry-run"]);
    assert_success(&dry_run);
    assert!(String::from_utf8_lossy(&dry_run.stdout).contains("tool (linux-x86_64)"));
    assert_eq!(fs::read_to_string(&auto_path).unwrap(), original);

    assert_success(&run_inro(home, &["clean", "--assets", "missing"]));
    assert_eq!(fs::read_to_string(&auto_path).unwrap(), original);
    assert_success(&run_inro(home, &["clean", "--assets", "tool"]));
    let auto: toml::Table = toml::from_str(&fs::read_to_string(&auto_path).unwrap()).unwrap();
    assert!(!auto.contains_key("tool"));
    assert!(auto.contains_key("other"));
    assert_success(&run_inro(home, &["clean", "--assets"]));
    let auto: toml::Table = toml::from_str(&fs::read_to_string(auto_path).unwrap()).unwrap();
    assert!(auto.is_empty());
    assert_eq!(fs::read_to_string(home.join("registry.d/local.toml")).unwrap(), user);
    assert_eq!(
        fs::read_to_string(home.join("registry/00-default.toml")).unwrap(),
        "upstream fixture"
    );
    assert_eq!(fs::read_to_string(home.join("pkgs/tool/v1/tool")).unwrap(), "keep binary");
    assert_eq!(
        fs::read_to_string(home.join("manifest.json")).unwrap(),
        "invalid manifest, unused for asset cleanup"
    );
    assert!(!run_inro(home, &["clean", "tool"]).status.success());
}

#[test]
fn plain_clean_preserves_remembered_asset_selections() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("registry")).unwrap();
    let auto_path = temp.path().join("registry/auto.toml");
    let content = "[tool.remote.github.asset]\nlinux-x86_64 = \"chosen.zip\"\n";
    fs::write(&auto_path, content).unwrap();
    assert_success(&run_inro(temp.path(), &["clean"]));
    assert_eq!(fs::read_to_string(auto_path).unwrap(), content);
}

#[test]
fn valid_auto_override_allows_loading_an_invalid_upstream_selector() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    fs::create_dir_all(home.join("registry")).unwrap();
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    fs::write(home.join("registry/00-default.toml"), format!(
        "[tool.remote.github]\nrepo = \"test/tool\"\n[tool.remote.github.asset]\n{platform} = 7\n"
    )).unwrap();
    let auto_path = home.join("registry/auto.toml");
    let auto = format!("[tool.remote.github.asset]\n{platform} = \"tool.zip\"\n");
    fs::write(&auto_path, &auto).unwrap();
    let output = run_inro(home, &["search", "tool"]);
    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("tool"));
    assert_eq!(fs::read_to_string(auto_path).unwrap(), auto);
}

#[cfg(unix)]
#[test]
fn invalid_upstream_selector_is_checked_only_when_auto_recovery_needs_it() {
    let temp = tempfile::tempdir().unwrap();
    let (home, _) = setup_learned_tool_home(temp.path());
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    fs::write(home.join("registry/00-default.toml"), format!(
        "[tool.remote.github]\nrepo = \"test/tool\"\n[tool.remote.github.asset]\n{platform} = 7\n"
    )).unwrap();
    let auto_path = home.join("registry/auto.toml");
    let auto_before = fs::read(&auto_path).unwrap();
    let manifest_before = fs::read(home.join("manifest.json")).unwrap();
    let (api_url, server) = serve_tool_release("v2.0.0", tool_binary(), false);
    let output =
        run_inro_with_env(&home, &["update", "tool"], &[("INRO_TEST_GITHUB_API_URL", &api_url)]);
    server.join().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid fallback asset selector"), "{stderr}");
    assert_eq!(fs::read(&auto_path).unwrap(), auto_before);
    assert_eq!(fs::read(home.join("manifest.json")).unwrap(), manifest_before);

    let valid_auto = format!("[tool.remote.github.asset]\n{platform} = \"tool\"\n");
    fs::write(&auto_path, &valid_auto).unwrap();
    let (api_url, server) = serve_tool_release("v2.0.0", tool_binary(), true);
    let output =
        run_inro_with_env(&home, &["update", "tool"], &[("INRO_TEST_GITHUB_API_URL", &api_url)]);
    assert_success(&output);
    server.join().unwrap();
    assert!(!String::from_utf8_lossy(&output.stderr).contains("no longer matches"));
    assert_eq!(fs::read_to_string(auto_path).unwrap(), valid_auto);
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

    let output = run_inro_with_env(
        &home,
        &["-v", "install", "tool"],
        &[("INRO_TEST_GITHUB_API_URL", &api_url)],
    );
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
fn direct_install_uses_exact_configured_version_and_normal_workflow() {
    let temp = tempfile::tempdir().unwrap();
    let (asset_url, server) = serve_direct_asset(tool_binary());
    let unavailable_url = format!("{}/unavailable-tool", closed_local_url());
    let (home, bin_dir) =
        setup_direct_tool_home(temp.path(), &[("1.0.0", &asset_url), ("2.0.0", &unavailable_url)]);

    let output = run_inro(&home, &["install", "tool@1.0.0"]);
    server.join().unwrap();

    assert_success(&output);
    let install_dir = home.join("pkgs/tool/1.0.0");
    assert_eq!(fs::read(install_dir.join("tool")).unwrap(), tool_binary());
    assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), install_dir.join("tool"));

    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(install_dir.join("inro-receipt.json")).unwrap()).unwrap();
    assert_eq!(receipt["version"], "1.0.0");
    assert!(receipt["remote"]["direct"]["1.0.0"].is_object());

    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["packages"]["tool"]["current_version"], "1.0.0");
    assert_eq!(manifest["packages"]["tool"]["versions"]["1.0.0"], receipt);
}

#[cfg(unix)]
#[test]
fn direct_install_requires_a_version_when_multiple_are_configured() {
    let temp = tempfile::tempdir().unwrap();
    let unavailable_url = format!("{}/tool", closed_local_url());
    let (home, _) = setup_direct_tool_home(
        temp.path(),
        &[("1.0.0", &unavailable_url), ("2.0.0", &unavailable_url)],
    );

    let output = run_inro(&home, &["-v", "install", "tool"]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("specify an exact version"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    if home.join("manifest.json").exists() {
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(home.join("manifest.json")).unwrap()).unwrap();
        assert!(manifest["packages"].get("tool").is_none());
    }
}

#[cfg(unix)]
#[test]
fn direct_update_without_version_skips_ambiguous_versions() {
    let temp = tempfile::tempdir().unwrap();
    let (asset_url, server) = serve_direct_asset(tool_binary());
    let unavailable_url = format!("{}/unavailable-tool", closed_local_url());
    let (home, bin_dir) =
        setup_direct_tool_home(temp.path(), &[("1.0.0", &asset_url), ("2.0.0", &unavailable_url)]);
    let installed = run_inro(&home, &["install", "tool@1.0.0"]);
    server.join().unwrap();
    assert_success(&installed);
    let manifest_before = fs::read(home.join("manifest.json")).unwrap();
    let link_before = fs::read_link(bin_dir.join("tool")).unwrap();

    for args in
        [vec!["update"], vec!["-v", "update"], vec!["update", "tool"], vec!["update", "--force"]]
    {
        let output = run_inro(&home, &args);
        assert_success(&output);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("skipping"), "{stderr}");
        assert!(stderr.contains("inro update tool@<version>"), "{stderr}");
        assert!(!stderr.contains("failed"), "{stderr}");
        assert_eq!(fs::read(home.join("manifest.json")).unwrap(), manifest_before);
        assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), link_before);
    }
}

#[cfg(unix)]
#[test]
fn direct_update_exact_version_still_updates_and_reports_real_errors() {
    let temp = tempfile::tempdir().unwrap();
    let (first_url, first_server) = serve_direct_asset(tool_binary());
    let (second_url, second_server) = serve_direct_asset(tool_binary());
    let unavailable_url = format!("{}/tool", closed_local_url());
    let (home, bin_dir) = setup_direct_tool_home(
        temp.path(),
        &[("1.0.0", &first_url), ("2.0.0", &second_url), ("3.0.0", &unavailable_url)],
    );
    let installed = run_inro(&home, &["install", "tool@1.0.0"]);
    first_server.join().unwrap();
    assert_success(&installed);
    let updated = run_inro(&home, &["update", "tool@2.0.0"]);
    second_server.join().unwrap();
    assert_success(&updated);
    let manifest_before = fs::read(home.join("manifest.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_before).unwrap();
    assert_eq!(manifest["packages"]["tool"]["current_version"], "2.0.0");
    assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), home.join("pkgs/tool/2.0.0/tool"));

    assert_success(&run_inro(&home, &["update", "tool@2.0.0"]));
    for (version, error) in [("tool@unknown", "not defined"), ("tool@3.0.0", "Download failed")] {
        let output = run_inro(&home, &["-v", "update", version]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains(error), "{output:?}");
        assert_eq!(fs::read(home.join("manifest.json")).unwrap(), manifest_before);
    }
}

#[cfg(unix)]
#[test]
fn direct_update_requires_a_version_after_registry_replaces_the_installed_version() {
    let temp = tempfile::tempdir().unwrap();
    let (first_url, first_server) = serve_direct_asset(tool_binary());
    let (home, bin_dir) = setup_direct_tool_home(temp.path(), &[("1.0.0", &first_url)]);
    let installed = run_inro(&home, &["install", "tool"]);
    first_server.join().unwrap();
    assert_success(&installed);
    let manifest_before = fs::read(home.join("manifest.json")).unwrap();
    let skipped = run_inro(&home, &["update"]);
    assert_success(&skipped);
    assert!(String::from_utf8_lossy(&skipped.stderr).contains("inro update tool@<version>"));
    assert_eq!(fs::read(home.join("manifest.json")).unwrap(), manifest_before);

    let (second_url, second_server) = serve_direct_asset(tool_binary());
    setup_direct_tool_home(temp.path(), &[("2.0.0", &second_url)]);
    for args in [vec!["update"], vec!["update", "tool"], vec!["update", "--force"]] {
        let skipped = run_inro(&home, &args);
        assert_success(&skipped);
        assert!(String::from_utf8_lossy(&skipped.stderr).contains("inro update tool@<version>"));
        assert_eq!(fs::read(home.join("manifest.json")).unwrap(), manifest_before);
        assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), home.join("pkgs/tool/1.0.0/tool"));
    }
    let exported = run_inro(&home, &["export"]);
    assert_success(&exported);
    assert!(String::from_utf8_lossy(&exported.stdout).contains("tool@1.0.0"));
    assert_success(&run_inro(&home, &["use", "tool", "1.0.0"]));
    let missing_version = run_inro(&home, &["-v", "install", "tool@1.0.0"]);
    assert!(!missing_version.status.success());
    assert!(String::from_utf8_lossy(&missing_version.stderr).contains("not defined"));

    let updated = run_inro(&home, &["update", "tool@2.0.0"]);
    second_server.join().unwrap();
    assert_success(&updated);
    assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), home.join("pkgs/tool/2.0.0/tool"));
    assert_success(&run_inro(&home, &["use", "tool", "1.0.0"]));
    assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), home.join("pkgs/tool/1.0.0/tool"));
    assert_success(&run_inro(&home, &["uninstall", "tool@1.0.0"]));
    assert!(!home.join("pkgs/tool/1.0.0").exists());
    assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), home.join("pkgs/tool/2.0.0/tool"));
}

#[cfg(unix)]
#[test]
fn update_skips_ambiguous_direct_packages_while_updating_github_packages() {
    let temp = tempfile::tempdir().unwrap();
    let (home, bin_dir) = setup_tool_home(temp.path());
    install_tool(&home, "v1.0.0");
    fs::write(
        home.join("registry.d/mysql.toml"),
        r#"
[mysql.remote.direct."8.4.11"]
macos-aarch64 = "http://127.0.0.1:1/mysql.tar.gz"
[mysql.remote.direct."9.7.2"]
macos-aarch64 = "http://127.0.0.1:1/mysql.tar.gz"
"#,
    )
    .unwrap();
    let mut mysql_receipt = manifest_receipt("mysql", "8.4.11");
    mysql_receipt["remote"] = serde_json::json!({ "direct": {
        "8.4.11": { "macos-aarch64": "http://127.0.0.1:1/mysql.tar.gz" },
        "9.7.2": { "macos-aarch64": "http://127.0.0.1:1/mysql.tar.gz" },
    } });
    let mysql_dir = home.join("pkgs/mysql/8.4.11");
    fs::create_dir_all(&mysql_dir).unwrap();
    fs::write(mysql_dir.join("mysql"), tool_binary()).unwrap();
    std::os::unix::fs::symlink(mysql_dir.join("mysql"), bin_dir.join("mysql")).unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("manifest.json")).unwrap()).unwrap();
    manifest["packages"]["mysql"] = serde_json::json!({
        "current_version": "8.4.11",
        "versions": { "8.4.11": mysql_receipt },
        "pinned": false,
    });
    let mysql_before = manifest["packages"]["mysql"].clone();
    fs::write(home.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
    let (api_url, server) = serve_tool_release("v2.0.0", tool_binary(), true);

    let output =
        run_inro_with_env(&home, &["-v", "update"], &[("INRO_TEST_GITHUB_API_URL", &api_url)]);
    server.join().unwrap();

    assert_success(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Updated 1 package(s)"), "{stderr}");
    assert!(stderr.contains("Skipped 1 package(s)"), "{stderr}");
    assert!(!stderr.contains("failed"), "{stderr}");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["packages"]["mysql"], mysql_before);
    assert_eq!(manifest["packages"]["tool"]["current_version"], "v2.0.0");
    assert_eq!(fs::read_link(bin_dir.join("mysql")).unwrap(), mysql_dir.join("mysql"));

    let invalid = run_inro(&home, &["-v", "update", "mysql", "not-installed"]);
    assert!(!invalid.status.success());
    let stderr = String::from_utf8_lossy(&invalid.stderr);
    assert!(stderr.contains("Skipped 1 package(s)"), "{stderr}");
    assert!(stderr.contains("1 package(s) failed to update"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn install_falls_back_to_a_release_page_when_latest_has_no_assets() {
    let temp = tempfile::tempdir().unwrap();
    let (home, bin_dir) = setup_tool_home(temp.path());
    let (api_url, server) = serve_tool_release_with_fallback("v2.0.0", tool_binary());

    let output =
        run_inro_with_env(&home, &["install", "tool"], &[("INRO_TEST_GITHUB_API_URL", &api_url)]);
    server.join().unwrap();

    assert_success(&output);
    assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), home.join("pkgs/tool/v2.0.0/tool"));
}

#[cfg(unix)]
#[test]
fn install_scans_a_second_release_page_when_the_first_has_no_assets() {
    let temp = tempfile::tempdir().unwrap();
    let (home, bin_dir) = setup_tool_home(temp.path());
    let (api_url, server) = serve_tool_release_on_second_page("v2.0.0", tool_binary());

    let output =
        run_inro_with_env(&home, &["install", "tool"], &[("INRO_TEST_GITHUB_API_URL", &api_url)]);
    server.join().unwrap();

    assert_success(&output);
    assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), home.join("pkgs/tool/v2.0.0/tool"));
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
        &[("INRO_TEST_GITHUB_API_URL", &api_url)],
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
        run_inro_with_env(&home, &["update", "tool"], &[("INRO_TEST_GITHUB_API_URL", &v2_api_url)]);
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
    let update = run_inro_with_env(
        &home,
        &["update", "tool"],
        &[("INRO_TEST_GITHUB_API_URL", &update_api_url)],
    );
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
        &[("INRO_TEST_GITHUB_API_URL", &unavailable_api)],
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
        &[("INRO_TEST_GITHUB_API_URL", &update_api_url)],
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

#[cfg(unix)]
#[test]
fn update_aligns_skipped_and_installed_package_statuses() {
    let temp = tempfile::tempdir().unwrap();
    let (home, _) = setup_tool_home(temp.path());
    install_tool(&home, "v1.0.0");

    let skipped_name = "very-long-pinned-package";
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("manifest.json")).unwrap()).unwrap();
    let mut skipped_state = manifest["packages"]["tool"].clone();
    skipped_state["pinned"] = serde_json::json!(true);
    manifest["packages"].as_object_mut().unwrap().insert(skipped_name.to_string(), skipped_state);
    fs::write(home.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();

    let (api_url, server) = serve_tool_release("v2.0.0", tool_binary(), true);
    let update = run_inro_with_env(
        &home,
        &["-v", "update", skipped_name, "tool"],
        &[("INRO_TEST_GITHUB_API_URL", &api_url)],
    );
    server.join().unwrap();

    assert_success(&update);
    let stderr = String::from_utf8_lossy(&update.stderr);
    let skipped_line = stderr.lines().find(|line| line.contains("pinned, skipping")).unwrap();
    let installed_line =
        stderr.lines().find(|line| line.contains("tool") && line.contains("v2.0.0")).unwrap();
    assert_eq!(
        skipped_line.split("pinned, skipping").next().unwrap().chars().count(),
        installed_line.split("v2.0.0").next().unwrap().chars().count(),
        "status columns should align:\n{stderr}"
    );
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

#[cfg(unix)]
#[test]
fn show_fetches_only_the_recent_release_page() {
    let temp = tempfile::tempdir().unwrap();
    let (home, _) = setup_tool_home(temp.path());
    let (api_url, server) = serve_tool_release_list("v1.0.0");

    let output =
        run_inro_with_env(&home, &["show", "tool"], &[("INRO_TEST_GITHUB_API_URL", &api_url)]);
    server.join().unwrap();

    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("v1.0.0"));
    assert_eq!(stdout.lines().filter(|line| line.starts_with("  - ")).count(), 5);
    assert!(stdout.contains("... (and more, check the remote for details)"));
}

#[cfg(unix)]
#[test]
fn show_lists_direct_versions_without_network_access() {
    let temp = tempfile::tempdir().unwrap();
    let unavailable_url = format!("{}/tool", closed_local_url());
    let (home, _) = setup_direct_tool_home(
        temp.path(),
        &[("1.0.0", &unavailable_url), ("2.0.0", &unavailable_url)],
    );

    let output = run_inro(&home, &["show", "tool"]);

    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("Source:     direct"));
    assert!(stderr.contains("Configured versions:"));
    assert!(stdout.contains("  - 1.0.0"), "stdout: {stdout}");
    assert!(stdout.contains("  - 2.0.0"), "stdout: {stdout}");
    assert!(!stderr.contains("Fetching remote info"));
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

    let output = run_inro_with_env(
        &home,
        &["-v", "update"],
        &[("INRO_TEST_GITHUB_API_URL", &unavailable_api)],
    );

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
fn export_writes_sorted_active_package_specs_and_skips_unlinked_packages() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("inro");
    fs::create_dir_all(&home).unwrap();
    let manifest = serde_json::json!({
        "schema_version": 2,
        "packages": {
            "zeta": {
                "current_version": "v2.0.0",
                "versions": {
                    "v1.0.0": manifest_receipt("zeta", "v1.0.0"),
                    "v2.0.0": manifest_receipt("zeta", "v2.0.0")
                },
                "pinned": false
            },
            "alpha": {
                "current_version": "v1.5.0",
                "versions": { "v1.5.0": manifest_receipt("alpha", "v1.5.0") },
                "pinned": true
            },
            "git": {
                "current_version": "v2.50.0",
                "versions": { "v2.50.0": manifest_receipt("git", "v2.50.0") },
                "pinned": false
            },
            "git-lfs": {
                "current_version": "v3.7.0",
                "versions": { "v3.7.0": manifest_receipt("git-lfs", "v3.7.0") },
                "pinned": false
            },
            "dormant": {
                "current_version": null,
                "versions": { "v3.0.0": manifest_receipt("dormant", "v3.0.0") },
                "pinned": false
            }
        }
    });
    fs::write(home.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();

    let output = run_inro(&home, &["export"]);

    assert_success(&output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "alpha@v1.5.0\ngit@v2.50.0\ngit-lfs@v3.7.0\nzeta@v2.0.0\n"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("Skipped 1 unlinked package"));
}

#[test]
fn export_writes_package_specs_atomically_to_an_output_file() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("inro");
    fs::create_dir_all(&home).unwrap();
    let manifest = serde_json::json!({
        "schema_version": 2,
        "packages": {
            "tool": {
                "current_version": "v1.0.0",
                "versions": { "v1.0.0": manifest_receipt("tool", "v1.0.0") },
                "pinned": false
            }
        }
    });
    fs::write(home.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    let output_path = temp.path().join("exports/packages.txt");

    let output = run_inro(&home, &["export", "--output", output_path.to_str().unwrap()]);

    assert_success(&output);
    assert!(output.stdout.is_empty());
    assert_eq!(fs::read_to_string(&output_path).unwrap(), "tool@v1.0.0\n");
    let export_dir_entries = fs::read_dir(output_path.parent().unwrap()).unwrap().count();
    assert_eq!(export_dir_entries, 1, "temporary export file was left behind");
}

#[cfg(unix)]
#[test]
fn import_installs_exact_versions_from_a_package_set_file() {
    let temp = tempfile::tempdir().unwrap();
    let (home, bin_dir) = setup_tool_home(temp.path());
    let package_set = temp.path().join("inro-packages.txt");
    fs::write(&package_set, "# workstation tools\n\n  tool@v1.0.0  \n").unwrap();
    let (api_url, server) = serve_tool_tag_release("v1.0.0", tool_binary(), true);

    let output = run_inro_with_env(
        &home,
        &["import", package_set.to_str().unwrap()],
        &[("INRO_TEST_GITHUB_API_URL", &api_url)],
    );
    server.join().unwrap();

    assert_success(&output);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["packages"]["tool"]["current_version"], "v1.0.0");
    assert_eq!(fs::read_link(bin_dir.join("tool")).unwrap(), home.join("pkgs/tool/v1.0.0/tool"));
}

#[test]
fn import_rejects_malformed_package_specs_before_installing() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("inro");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("config.toml"), "upstreams = []\n").unwrap();
    let package_set = temp.path().join("invalid-packages.txt");
    fs::write(&package_set, "tool@\n").unwrap();

    let output = run_inro(&home, &["import", package_set.to_str().unwrap()]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Invalid package specification 'tool@'"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!home.join("manifest.json").exists());
}

#[test]
fn import_requires_an_exact_version_for_every_package() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("inro");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("config.toml"), "upstreams = []\n").unwrap();
    let package_set = temp.path().join("unversioned-packages.txt");
    fs::write(&package_set, "tool\n").unwrap();

    let output = run_inro(&home, &["import", package_set.to_str().unwrap()]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("Invalid package specification 'tool': an exact version is required"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!home.join("manifest.json").exists());
}

#[cfg(unix)]
#[test]
fn source_edit_opens_and_validates_the_default_local_registry() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("inro");
    let editor = temp.path().join("fake-editor");
    fs::write(
        &editor,
        "#!/bin/sh\nprintf '[custom.remote.github]\\nrepo = \"test/custom\"\\n' > \"$1\"\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&editor).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&editor, permissions).unwrap();

    let output =
        run_inro_with_env(&home, &["source", "edit"], &[("VISUAL", editor.to_str().unwrap())]);

    assert_success(&output);
    assert_eq!(
        fs::read_to_string(home.join("registry.d/local.toml")).unwrap(),
        "[custom.remote.github]\nrepo = \"test/custom\"\n"
    );
}

#[cfg(unix)]
#[test]
fn source_edit_falls_back_to_vi_without_editor_environment_variables() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("inro");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let nano = bin_dir.join("nano");
    fs::write(&nano, "#!/bin/sh\nexit 23\n").unwrap();
    let vi = bin_dir.join("vi");
    fs::write(
        &vi,
        "#!/bin/sh\nprintf '[fallback.remote.github]\\nrepo = \"test/fallback\"\\n' > \"$1\"\n",
    )
    .unwrap();
    for editor in [&nano, &vi] {
        let mut permissions = fs::metadata(editor).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(editor, permissions).unwrap();
    }

    let output = run_inro_with_env(
        &home,
        &["source", "edit", "fallback"],
        &[("PATH", bin_dir.to_str().unwrap())],
    );

    assert_success(&output);
    assert_eq!(
        fs::read_to_string(home.join("registry.d/fallback.toml")).unwrap(),
        "[fallback.remote.github]\nrepo = \"test/fallback\"\n"
    );
}

#[cfg(unix)]
#[test]
fn source_edit_keeps_the_live_registry_valid_when_the_edit_is_invalid() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("inro");
    let registry_dir = home.join("registry.d");
    fs::create_dir_all(&registry_dir).unwrap();
    fs::write(home.join("config.toml"), "upstreams = []\n").unwrap();
    let source_path = registry_dir.join("local.toml");
    let original = "[stable.remote.github]\nrepo = \"test/stable\"\n";
    fs::write(&source_path, original).unwrap();

    let editor = temp.path().join("invalid-editor");
    fs::write(&editor, "#!/bin/sh\nprintf '[broken\\n' > \"$1\"\n").unwrap();
    let mut permissions = fs::metadata(&editor).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&editor, permissions).unwrap();

    let edit =
        run_inro_with_env(&home, &["source", "edit"], &[("VISUAL", editor.to_str().unwrap())]);

    assert!(!edit.status.success());
    assert!(
        String::from_utf8_lossy(&edit.stderr).contains("Edited local source is invalid"),
        "stderr: {}",
        String::from_utf8_lossy(&edit.stderr)
    );
    assert_eq!(fs::read_to_string(&source_path).unwrap(), original);

    let search = run_inro(&home, &["search", "stable"]);
    assert_success(&search);
    assert!(String::from_utf8_lossy(&search.stdout).contains("stable"));
}

#[cfg(unix)]
#[test]
fn source_edit_does_not_overwrite_a_concurrent_source_change() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("inro");
    let registry_dir = home.join("registry.d");
    fs::create_dir_all(&registry_dir).unwrap();
    fs::write(home.join("config.toml"), "upstreams = []\n").unwrap();
    let source_path = registry_dir.join("local.toml");
    fs::write(&source_path, "[original.remote.github]\nrepo = \"test/original\"\n").unwrap();

    let ready = temp.path().join("editor-ready");
    let release = temp.path().join("editor-release");
    let editor = temp.path().join("waiting-editor");
    fs::write(
        &editor,
        "#!/bin/sh\nprintf '[staged.remote.github]\\nrepo = \"test/staged\"\\n' > \"$3\"\ntouch \"$1\"\nwhile [ ! -e \"$2\" ]; do sleep 0.05; done\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&editor).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&editor, permissions).unwrap();
    let visual = format!("{} {} {}", editor.display(), ready.display(), release.display());

    let mut command = inro_command_with_env(&home, &["source", "edit"], &[("VISUAL", &visual)]);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut edit = command.spawn().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(std::time::Instant::now() < deadline, "editor did not start");
        assert!(edit.try_wait().unwrap().is_none(), "source edit exited before editor was ready");
        thread::sleep(Duration::from_millis(10));
    }

    let concurrent = "[concurrent.remote.github]\nrepo = \"test/concurrent\"\n";
    fs::write(&source_path, concurrent).unwrap();
    fs::write(&release, b"").unwrap();
    let output = edit.wait_with_output().unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("changed while it was being edited"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(&source_path).unwrap(), concurrent);
}

#[test]
fn test_server_stops_waiting_without_a_connection() {
    let started = std::time::Instant::now();
    let (_url, server) = serve_once_with_timeout("unused", Duration::from_millis(50));

    server.join().unwrap();

    assert!(started.elapsed() < Duration::from_secs(1));
}

#[cfg(unix)]
mod relink {
    use std::os::unix::fs::symlink;

    use super::*;

    fn setup(root: &Path) -> (PathBuf, PathBuf) {
        let (home, bin_dir) = setup_tool_home(root);
        fs::write(home.join("config.toml"), format!("bin_dir = {:?}\n", bin_dir)).unwrap();
        fs::remove_file(home.join("registry.d/tool.toml")).unwrap();
        fs::create_dir_all(home.join("registry")).unwrap();
        fs::write(
            home.join("registry/00-default.toml"),
            r#"
[tool]
bin = [{ name = "tool" }, { name = "stale" }]
[tool.remote.direct."v2.0.0"]
"unsupported-platform" = "http://127.0.0.1:1/tool.tar.gz"
"#,
        )
        .unwrap();
        let install_dir = home.join("pkgs/tool/v1.0.0");
        fs::create_dir_all(install_dir.join("bin")).unwrap();
        fs::create_dir_all(&bin_dir).unwrap();
        for name in ["tool", "helper", "stale"] {
            fs::write(install_dir.join("bin").join(name), tool_binary()).unwrap();
        }
        let mut receipt = manifest_receipt("tool", "v1.0.0");
        receipt["binaries"] = serde_json::json!([
            { "name": "tool", "bin_subpath": "bin/tool" },
            { "name": "stale", "bin_subpath": "bin/stale" },
        ]);
        fs::write(install_dir.join("inro-receipt.json"), serde_json::to_vec(&receipt).unwrap())
            .unwrap();
        fs::write(
            home.join("manifest.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 2,
                "packages": { "tool": {
                    "current_version": "v1.0.0", "pinned": true,
                    "versions": { "v1.0.0": receipt },
                } },
            }))
            .unwrap(),
        )
        .unwrap();
        symlink(install_dir.join("bin/tool"), bin_dir.join("tool")).unwrap();
        symlink("../inro/pkgs/tool/v1.0.0/bin/stale", bin_dir.join("stale")).unwrap();
        (home, bin_dir)
    }

    fn set_bins(home: &Path, bins: &str) {
        fs::write(home.join("registry.d/local.toml"), format!("[tool]\nbin = {bins}\n")).unwrap();
    }

    fn json(path: &Path) -> serde_json::Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    fn assert_absent(path: &Path) {
        assert!(fs::symlink_metadata(path).is_err(), "unexpected entry: {}", path.display());
    }

    #[test]
    fn relink_applies_local_bins_offline_and_use_preserves_the_new_record() {
        let temp = tempfile::tempdir().unwrap();
        let (home, bin_dir) = setup(temp.path());
        let install_dir = home.join("pkgs/tool/v1.0.0");
        let receipt_path = install_dir.join("inro-receipt.json");
        let mut expected = json(&receipt_path);
        set_bins(&home, r#"[{name="tool", link="renamed"}, {name="helper"}]"#);

        assert_success(&run_inro(&home, &["relink", "tool"]));

        assert_eq!(fs::read_link(bin_dir.join("renamed")).unwrap(), install_dir.join("bin/tool"));
        assert_eq!(fs::read_link(bin_dir.join("helper")).unwrap(), install_dir.join("bin/helper"));
        assert_absent(&bin_dir.join("tool"));
        assert_absent(&bin_dir.join("stale"));
        assert_eq!(fs::read(install_dir.join("bin/stale")).unwrap(), tool_binary());
        expected["binaries"] = serde_json::json!([
            { "name": "renamed", "bin_subpath": "bin/tool" },
            { "name": "helper", "bin_subpath": "bin/helper" },
        ]);
        assert_eq!(json(&receipt_path), expected);
        let manifest = json(&home.join("manifest.json"));
        assert_eq!(manifest["packages"]["tool"]["versions"]["v1.0.0"], expected);
        assert_eq!(manifest["packages"]["tool"]["current_version"], "v1.0.0");
        assert_eq!(manifest["packages"]["tool"]["pinned"], true);

        assert_success(&run_inro(&home, &["unlink", "tool"]));
        assert_absent(&bin_dir.join("renamed"));
        assert_absent(&bin_dir.join("helper"));
        set_bins(&home, r#"[{name="missing"}]"#);
        assert_success(&run_inro(&home, &["use", "tool", "v1.0.0"]));
        assert!(bin_dir.join("renamed").exists());
        assert!(bin_dir.join("helper").exists());
        assert_absent(&bin_dir.join("stale"));
    }

    #[test]
    fn relink_repairs_missing_and_broken_links_with_unchanged_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let (home, bin_dir) = setup(temp.path());
        let receipt_path = home.join("pkgs/tool/v1.0.0/inro-receipt.json");
        let before = json(&receipt_path);
        fs::remove_file(bin_dir.join("tool")).unwrap();
        fs::remove_file(bin_dir.join("stale")).unwrap();
        symlink(home.join("pkgs/tool/missing/stale"), bin_dir.join("stale")).unwrap();

        assert_success(&run_inro(&home, &["relink", "tool"]));
        assert_success(&run_inro(&home, &["relink", "tool"]));

        assert_eq!(json(&receipt_path), before);
        assert_eq!(
            fs::read_link(bin_dir.join("tool")).unwrap(),
            home.join("pkgs/tool/v1.0.0/bin/tool")
        );
        assert_eq!(
            fs::read_link(bin_dir.join("stale")).unwrap(),
            home.join("pkgs/tool/v1.0.0/bin/stale")
        );
    }

    #[test]
    fn relink_rejects_invalid_definitions_before_changing_links_or_records() {
        for (bins, error) in [
            (r#"[{name="helper"}, {name="missing"}]"#, "Binary 'missing' not found"),
            (r#"[{name="helper",link="tool"}, {name="tool"}]"#, "Duplicate link name"),
            (r#"[{name="helper",link="../outside"}]"#, "link name"),
            (r#"[{name="../helper"}]"#, "binary name"),
            (r#"[{name={"unsupported-platform"="helper"}}]"#, "no binary defined"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let (home, bin_dir) = setup(temp.path());
            let before = fs::read(home.join("manifest.json")).unwrap();
            let receipt_path = home.join("pkgs/tool/v1.0.0/inro-receipt.json");
            let receipt_before = fs::read(&receipt_path).unwrap();
            let stale_before = fs::read_link(bin_dir.join("stale")).unwrap();
            set_bins(&home, bins);

            let result = run_inro(&home, &["relink", "tool"]);

            assert!(!result.status.success());
            assert!(String::from_utf8_lossy(&result.stderr).contains(error), "{result:?}");
            assert_eq!(fs::read(home.join("manifest.json")).unwrap(), before);
            assert_eq!(fs::read(receipt_path).unwrap(), receipt_before);
            assert_eq!(fs::read_link(bin_dir.join("stale")).unwrap(), stale_before);
            assert!(bin_dir.join("tool").exists());
            assert_absent(&bin_dir.join("helper"));
        }
    }

    #[test]
    fn relink_refuses_files_foreign_symlinks_and_other_packages_before_changing_links() {
        for kind in ["file", "foreign-link", "other-package"] {
            let temp = tempfile::tempdir().unwrap();
            let (home, bin_dir) = setup(temp.path());
            let occupied = bin_dir.join("occupied");
            match kind {
                "file" => fs::write(&occupied, b"user file").unwrap(),
                "foreign-link" => symlink(temp.path().join("user-tool"), &occupied).unwrap(),
                _ => symlink(home.join("pkgs/other/v1/tool"), &occupied).unwrap(),
            }
            set_bins(&home, r#"[{name="helper"}, {name="tool",link="occupied"}]"#);
            let before = fs::read(home.join("manifest.json")).unwrap();
            let stale_before = fs::read_link(bin_dir.join("stale")).unwrap();

            let result = run_inro(&home, &["relink", "tool"]);

            assert!(!result.status.success());
            assert!(String::from_utf8_lossy(&result.stderr).contains("Refusing"), "{result:?}");
            assert_eq!(fs::read(home.join("manifest.json")).unwrap(), before);
            assert_eq!(fs::read_link(bin_dir.join("stale")).unwrap(), stale_before);
            assert!(bin_dir.join("tool").exists());
            assert_absent(&bin_dir.join("helper"));
            if kind == "file" {
                assert_eq!(fs::read(&occupied).unwrap(), b"user file");
            } else {
                assert!(occupied.is_symlink());
            }
        }
    }

    #[test]
    fn relink_leaves_obsolete_destinations_replaced_by_users_or_other_packages_alone() {
        let temp = tempfile::tempdir().unwrap();
        let (home, bin_dir) = setup(temp.path());
        fs::remove_file(bin_dir.join("tool")).unwrap();
        fs::write(bin_dir.join("tool"), b"user file").unwrap();
        fs::remove_file(bin_dir.join("stale")).unwrap();
        let foreign = home.join("pkgs/other/v1/stale");
        symlink(&foreign, bin_dir.join("stale")).unwrap();
        set_bins(&home, r#"[{name="helper"}]"#);

        assert_success(&run_inro(&home, &["relink", "tool"]));

        assert_eq!(fs::read(bin_dir.join("tool")).unwrap(), b"user file");
        assert_eq!(fs::read_link(bin_dir.join("stale")).unwrap(), foreign);
        assert!(bin_dir.join("helper").exists());
    }

    #[test]
    fn relink_manifest_failure_restores_exact_links_and_receipt_state() {
        for missing_receipt in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let (home, bin_dir) = setup(temp.path());
            let receipt_path = home.join("pkgs/tool/v1.0.0/inro-receipt.json");
            let receipt_before = fs::read(&receipt_path).unwrap();
            if missing_receipt {
                fs::remove_file(&receipt_path).unwrap();
            }
            let before = fs::read(home.join("manifest.json")).unwrap();
            let stale_before = fs::read_link(bin_dir.join("stale")).unwrap();
            fs::remove_file(bin_dir.join("tool")).unwrap();
            let helper_before = home.join("pkgs/tool/v0/helper");
            symlink(&helper_before, bin_dir.join("helper")).unwrap();
            set_bins(&home, r#"[{name="helper"}, {name="tool"}, {name="tool",link="extra"}]"#);
            fs::create_dir(home.join("manifest.tmp")).unwrap();

            let result = run_inro(&home, &["relink", "tool"]);

            assert!(!result.status.success());
            assert_eq!(fs::read(home.join("manifest.json")).unwrap(), before);
            if missing_receipt {
                assert_absent(&receipt_path);
            } else {
                assert_eq!(fs::read(&receipt_path).unwrap(), receipt_before);
            }
            assert_eq!(fs::read_link(bin_dir.join("stale")).unwrap(), stale_before);
            assert_eq!(fs::read_link(bin_dir.join("helper")).unwrap(), helper_before);
            assert_absent(&bin_dir.join("tool"));
            assert_absent(&bin_dir.join("extra"));
            let count = fs::read_dir(home.join("pkgs/tool/v1.0.0")).unwrap().count();
            assert_eq!(count, if missing_receipt { 1 } else { 2 });
        }
    }

    #[test]
    fn relink_receipt_staging_failure_keeps_links_and_manifest_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let (home, bin_dir) = setup(temp.path());
        let receipt_path = home.join("pkgs/tool/v1.0.0/inro-receipt.json");
        fs::remove_file(&receipt_path).unwrap();
        fs::create_dir(&receipt_path).unwrap();
        let before = fs::read(home.join("manifest.json")).unwrap();
        let stale_before = fs::read_link(bin_dir.join("stale")).unwrap();
        set_bins(&home, r#"[{name="helper"}]"#);

        let result = run_inro(&home, &["relink", "tool"]);

        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("not a regular file"));
        assert_eq!(fs::read(home.join("manifest.json")).unwrap(), before);
        assert_eq!(fs::read_link(bin_dir.join("stale")).unwrap(), stale_before);
        assert!(bin_dir.join("tool").exists());
        assert_absent(&bin_dir.join("helper"));
    }

    #[test]
    fn relink_requires_an_installed_active_package_and_a_registry_definition() {
        let temp = tempfile::tempdir().unwrap();
        let (home, bin_dir) = setup(temp.path());
        let missing = run_inro(&home, &["relink", "missing"]);
        assert!(!missing.status.success());
        assert!(String::from_utf8_lossy(&missing.stderr).contains("is not installed"));

        assert_success(&run_inro(&home, &["unlink", "tool"]));
        let before = fs::read(home.join("manifest.json")).unwrap();
        let unlinked = run_inro(&home, &["relink", "tool"]);
        assert!(!unlinked.status.success());
        assert!(String::from_utf8_lossy(&unlinked.stderr).contains("inro use tool <VERSION>"));
        assert_eq!(fs::read(home.join("manifest.json")).unwrap(), before);
        assert_absent(&bin_dir.join("tool"));

        assert_success(&run_inro(&home, &["use", "tool", "v1.0.0"]));
        fs::write(
            home.join("registry/00-default.toml"),
            "[other.remote.github]\nrepo='test/other'\n",
        )
        .unwrap();
        let unregistered = run_inro(&home, &["relink", "tool"]);
        assert!(!unregistered.status.success());
        assert!(String::from_utf8_lossy(&unregistered.stderr).contains("not found in registry"));
        assert!(bin_dir.join("tool").exists());
    }
}
