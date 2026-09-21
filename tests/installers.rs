//! Download script integration tests. No external registry or executable
//! payload is used.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

// Deliberately not executable: the download scripts must never run this
// payload.
const PAYLOAD: &[u8] = b"downloaded inro fixture\n";
const EXE: &str = if cfg!(windows) { "inro.exe" } else { "inro" };
const ASSET: &str = if cfg!(windows) {
    "inro-windows-x86_64-msvc.zip"
} else if cfg!(target_os = "macos") {
    "inro-macos-aarch64-darwin.tar.xz"
} else {
    "inro-linux-x86_64-musl.tar.xz"
};
const TO: &str = if cfg!(windows) { "-InstallDir" } else { "--to" };
const VERSION: &str = if cfg!(windows) { "-Version" } else { "--version" };

#[derive(Default)]
struct ServerState {
    files: HashMap<String, Vec<u8>>,
    requests: Vec<String>,
    concurrent_file: Option<PathBuf>,
}

struct Fixture {
    root: tempfile::TempDir,
    address: std::net::SocketAddr,
    state: Arc<Mutex<ServerState>>,
    stop: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(ServerState::default()));
        let server_state = Arc::clone(&state);
        let server_stop = Arc::clone(&stop);
        let server = thread::spawn(move || {
            for stream in listener.incoming() {
                if server_stop.load(Ordering::Relaxed) {
                    break;
                }
                let mut stream = stream.unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        break;
                    }
                    request.push(byte[0]);
                    assert!(request.len() < 16 * 1024);
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap_or("");
                let body = {
                    let mut state = server_state.lock().unwrap();
                    state.requests.push(path.to_string());
                    if let Some(target) = &state.concurrent_file {
                        fs::write(target, b"created during download").unwrap();
                    }
                    state.files.get(path).cloned()
                };
                let status = if body.is_some() { "200 OK" } else { "404 Not Found" };
                let body = body.unwrap_or_default();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(&body);
            }
        });
        let fixture =
            Self { root: tempfile::tempdir().unwrap(), address, state, stop, server: Some(server) };
        fs::create_dir(fixture.path("state")).unwrap();
        fs::write(fixture.path("state/config.toml"), b"invalid config deliberately ignored")
            .unwrap();
        fs::write(fixture.path("state/manifest.json"), b"existing state").unwrap();
        fixture.release(EXE);
        fixture
    }

    fn path(&self, name: &str) -> PathBuf { self.root.path().join(name) }

    fn release(&self, member: &str) {
        let archive = make_archive(member);
        let path = self.path(ASSET);
        fs::write(&path, &archive).unwrap();
        // Use the checksum utility already required by the script, without
        // adding a crate.
        let digest = checksum(&path);
        let mut state = self.state.lock().unwrap();
        for prefix in ["/releases/latest/download", "/releases/download/v1.2.3"] {
            state.files.insert(format!("{prefix}/{ASSET}"), archive.clone());
            state.files.insert(
                format!("{prefix}/SHA256SUMS"),
                format!("{digest}  {ASSET}\n").into_bytes(),
            );
        }
    }

    fn checksums(&self, contents: &str) {
        let mut state = self.state.lock().unwrap();
        for (path, body) in &mut state.files {
            if path.ends_with("/SHA256SUMS") {
                *body = contents.as_bytes().to_vec();
            }
        }
    }

    fn request_count(&self) -> usize { self.state.lock().unwrap().requests.len() }

    fn invoke(&self, cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
        fs::create_dir_all(cwd).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut command = if cfg!(windows) {
            let mut command = Command::new("pwsh");
            command.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]);
            command.arg(repo.join("install.ps1"));
            command
        } else {
            let mut command = Command::new("sh");
            command.arg(repo.join("install.sh"));
            command
        };
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("INRO_") {
                command.env_remove(key);
            }
        }
        command
            .current_dir(cwd)
            .args(args)
            .env("INRO_RELEASES_URL", format!("http://{}/releases", self.address))
            .env("INRO_HOME", self.path("state"))
            .env("INRO_BIN_DIR", self.path("managed bin"))
            .env("NO_PROXY", "127.0.0.1,localhost")
            .env("no_proxy", "127.0.0.1,localhost")
            .envs(env.iter().copied());
        let output = run_with_timeout(&mut command);
        assert_eq!(
            fs::read(self.path("state/config.toml")).unwrap(),
            b"invalid config deliberately ignored"
        );
        assert_eq!(fs::read(self.path("state/manifest.json")).unwrap(), b"existing state");
        assert_eq!(fs::read_dir(self.path("state")).unwrap().count(), 2);
        assert!(!self.path("managed bin").exists());
        output
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(server) = self.server.take() {
            let result = server.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}

// Log to files so a subprocess cannot block on a full pipe before the timeout
// fires.
fn run_with_timeout(command: &mut Command) -> Output {
    let logs = tempfile::tempdir().unwrap();
    let stdout = logs.path().join("stdout");
    let stderr = logs.path().join("stderr");
    command
        .stdin(Stdio::null())
        .stdout(File::create(&stdout).unwrap())
        .stderr(File::create(&stderr).unwrap());
    let mut child = command.spawn().expect("script interpreter or checksum utility unavailable");
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out: {command:?}\n{}", fs::read_to_string(&stderr).unwrap());
        }
        thread::sleep(Duration::from_millis(10));
    };
    Output { status, stdout: fs::read(stdout).unwrap(), stderr: fs::read(stderr).unwrap() }
}

#[cfg(unix)]
fn make_archive(member: &str) -> Vec<u8> {
    let encoder = xz2::write::XzEncoder::new(Vec::new(), 0);
    let mut archive = tar::Builder::new(encoder);
    for (name, data) in [(member, PAYLOAD), ("README.md", b"not copied".as_slice())] {
        let mut header = tar::Header::new_gnu();
        header.set_path(name).unwrap();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive.append(&header, data).unwrap();
    }
    archive.into_inner().unwrap().finish().unwrap()
}

#[cfg(windows)]
fn make_archive(member: &str) -> Vec<u8> {
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in [(member, PAYLOAD), ("README.md", b"not copied".as_slice())] {
        archive.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        archive.write_all(data).unwrap();
    }
    archive.finish().unwrap().into_inner()
}

fn checksum(path: &Path) -> String {
    let mut command = if cfg!(windows) {
        let mut command = Command::new("pwsh");
        command.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-FileHash -Algorithm SHA256 -LiteralPath $env:INRO_TEST_ARCHIVE).Hash",
        ]);
        command.env("INRO_TEST_ARCHIVE", path);
        command
    } else if cfg!(target_os = "macos") {
        let mut command = Command::new("shasum");
        command.args(["-a", "256"]).arg(path);
        command
    } else {
        let mut command = Command::new("sha256sum");
        command.arg(path);
        command
    };
    let output = run_with_timeout(&mut command);
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().split_whitespace().next().unwrap().to_lowercase()
}

fn success(output: &Output) {
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{output:?}");
    for message in
        ["Downloaded and verified:", "source update", "install inro", "configured bin_dir"]
    {
        assert!(text.contains(message), "{text}");
    }
}

fn failure(output: &Output, message: &str) {
    assert!(!output.status.success(), "{output:?}");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains(message), "{text}");
}

#[test]
fn downloads_only_binary_to_current_directory_and_prints_usable_hints() {
    let fixture = Fixture::new();
    let cwd = fixture.path("download space's [dir]");
    let output = fixture.invoke(&cwd, &[], &[]);
    success(&output);
    let target = cwd.join(EXE);
    assert_eq!(fs::read(&target).unwrap(), PAYLOAD);
    assert!(!target.is_symlink());
    assert_eq!(fs::read_dir(&cwd).unwrap().count(), 1);
    let text = String::from_utf8(output.stdout).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(fs::metadata(&target).unwrap().permissions().mode() & 0o111, 0);
        let hint = text.lines().find(|line| line.ends_with(" --help")).unwrap();
        assert_eq!(
            shell_words::split(hint).unwrap(),
            [target.canonicalize().unwrap().to_string_lossy(), "--help".into()]
        );
    }
    #[cfg(windows)]
    assert!(text.contains(&target.to_string_lossy().replace('\'', "''")));
    let requests = fixture.request_count();
    failure(&fixture.invoke(&cwd, &[], &[]), "already exists");
    assert_eq!(fixture.request_count(), requests);
    assert_eq!(fs::read(target).unwrap(), PAYLOAD);
}

#[test]
fn download_options_override_environment_and_support_relative_directories() {
    let fixture = Fixture::new();
    let cwd = fixture.path("work");
    let unused = fixture.path("unused");
    success(&fixture.invoke(
        &cwd,
        &[VERSION, "v1.2.3", TO, "chosen directory"],
        &[("INRO_VERSION", "invalid"), ("INRO_INSTALL_DIR", unused.to_str().unwrap())],
    ));
    assert_eq!(fs::read(cwd.join("chosen directory").join(EXE)).unwrap(), PAYLOAD);
    assert!(!unused.exists());
    let output_dir = fixture.path("environment output");
    success(&fixture.invoke(
        &cwd,
        &[],
        &[("INRO_VERSION", "1.2.3"), ("INRO_INSTALL_DIR", output_dir.to_str().unwrap())],
    ));
    assert_eq!(fs::read(output_dir.join(EXE)).unwrap(), PAYLOAD);
    let state = fixture.state.lock().unwrap();
    assert!(state.requests.iter().all(|path| path.starts_with("/releases/download/v1.2.3/")));
    assert!(
        state.requests.iter().all(|path| path.ends_with(ASSET) || path.ends_with("SHA256SUMS"))
    );
}

#[test]
fn download_failures_leave_no_binary_or_staging_files() {
    let fixture = Fixture::new();
    fixture.checksums(&format!("{}  {ASSET}\n", "0".repeat(64)));
    let cwd = fixture.path("bad checksum");
    failure(&fixture.invoke(&cwd, &[], &[]), "checksum verification failed");
    assert_eq!(fs::read_dir(cwd).unwrap().count(), 0);
    fixture.checksums("no matching checksum\n");
    let cwd = fixture.path("missing checksum");
    failure(&fixture.invoke(&cwd, &[], &[]), "SHA256SUMS has no valid entry");
    assert_eq!(fs::read_dir(cwd).unwrap().count(), 0);
    fixture.release("missing-binary");
    let cwd = fixture.path("missing binary");
    failure(&fixture.invoke(&cwd, &[], &[]), "release archive does not contain");
    assert_eq!(fs::read_dir(cwd).unwrap().count(), 0);
    let requests = fixture.request_count();
    failure(
        &fixture.invoke(&fixture.path("invalid version"), &[VERSION, "../escape"], &[]),
        "invalid version",
    );
    assert_eq!(fixture.request_count(), requests);
}

#[test]
fn download_does_not_overwrite_file_created_during_download() {
    let fixture = Fixture::new();
    let cwd = fixture.path("concurrent file");
    fixture.state.lock().unwrap().concurrent_file = Some(cwd.join(EXE));
    let output = fixture.invoke(&cwd, &[], &[]);
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(fs::read(cwd.join(EXE)).unwrap(), b"created during download");
    assert_eq!(fs::read_dir(cwd).unwrap().count(), 1);
}

#[test]
fn download_refuses_directories_and_existing_or_dangling_links() {
    let fixture = Fixture::new();
    for kind in ["directory", "symlink", "dangling", "directory-link"] {
        let cwd = fixture.path(kind);
        fs::create_dir(&cwd).unwrap();
        let link = cwd.join(EXE);
        let original = fixture.path(&format!("original-{kind}"));
        if kind == "directory" {
            fs::create_dir(&link).unwrap();
        } else {
            if kind == "symlink" {
                fs::write(&original, b"untouched").unwrap();
            }
            if kind == "directory-link" {
                fs::create_dir(&original).unwrap();
            }
            #[cfg(unix)]
            let result = std::os::unix::fs::symlink(&original, &link);
            #[cfg(windows)]
            let result = if kind == "directory-link" {
                std::os::windows::fs::symlink_dir(&original, &link)
            } else {
                std::os::windows::fs::symlink_file(&original, &link)
            };
            if cfg!(windows)
                && result.as_ref().err().is_some_and(|e| e.raw_os_error() == Some(1314))
            {
                eprintln!("Skipping link fixture: Windows host lacks symbolic-link permissions");
                continue;
            }
            result.unwrap();
        }
        let requests = fixture.request_count();
        failure(&fixture.invoke(&cwd, &[], &[]), "already exists");
        assert_eq!(fixture.request_count(), requests);
        if kind != "directory" {
            assert_eq!(fs::read_link(&link).unwrap(), original);
        }
        if kind == "symlink" {
            assert_eq!(fs::read(&original).unwrap(), b"untouched");
        }
    }
}
