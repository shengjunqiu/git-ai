#![cfg(windows)]

use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use url::Url;

struct CommandOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn git_ai_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_git-ai"))
}

fn configure_isolated_home(command: &mut Command, home: &Path) {
    let drive = home
        .components()
        .next()
        .map(|component| component.as_os_str().to_os_string())
        .unwrap_or_default();
    let home_text = home.to_string_lossy();
    let home_path = home_text
        .strip_prefix(&drive.to_string_lossy().to_string())
        .unwrap_or(&home_text);
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("HOMEDRIVE", drive)
        .env("HOMEPATH", home_path)
        .env("GIT_AI_AUTH_KEYRING", "false")
        .env("GIT_AI_ASYNC_MODE", "false");
}

fn read_file(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn wait_for_authorization_url(stderr_path: &Path, timeout: Duration) -> Url {
    let deadline = Instant::now() + timeout;
    loop {
        let stderr = read_file(stderr_path);
        if let Some(raw) = stderr
            .lines()
            .map(str::trim)
            .find(|line| line.contains("/auth/cli/authorize?"))
        {
            return Url::parse(raw).expect("authorization output should contain a valid URL");
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for authorization URL\nstderr:\n{stderr}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_child(mut child: Child, timeout: Duration) -> ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("failed polling child") {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("command timed out after {timeout:?}");
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn send_callback_probe(redirect_uri: &Url) {
    let address = format!("127.0.0.1:{}", redirect_uri.port().expect("callback port"));
    drop(TcpStream::connect(address).expect("empty callback probe should connect"));
}

fn send_authorized_callback(redirect_uri: &Url, state: &str) {
    let mut callback = redirect_uri.clone();
    callback
        .query_pairs_mut()
        .append_pair("code", "mock-authorization-code")
        .append_pair("state", state);
    let address = format!("127.0.0.1:{}", callback.port().expect("callback port"));
    let mut stream = TcpStream::connect(address).expect("callback should connect");
    write!(
        stream,
        "GET {}?{} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        callback.path(),
        callback.query().expect("callback query")
    )
    .expect("failed writing callback");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("failed reading callback response");
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
}

fn base64_url(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::new();
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or(0) as u32;
        let third = chunk.get(2).copied().unwrap_or(0) as u32;
        let value = (first << 16) | (second << 8) | third;
        output.push(TABLE[((value >> 18) & 63) as usize] as char);
        output.push(TABLE[((value >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            output.push(TABLE[((value >> 6) & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            output.push(TABLE[(value & 63) as usize] as char);
        }
    }
    output
}

fn mock_access_token() -> String {
    let header = base64_url(br#"{"alg":"none","typ":"JWT"}"#);
    let claims = base64_url(
        serde_json::to_string(&json!({
            "sub": "windows-oauth-user",
            "email": "windows@example.test",
            "name": "Windows OAuth Test",
            "personal_org_id": "org-test",
            "orgs": []
        }))
        .unwrap()
        .as_bytes(),
    );
    format!("{header}.{claims}.test")
}

fn read_http_request(stream: &mut TcpStream) -> (String, Value) {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        if header == "\r\n" || header.is_empty() {
            break;
        }
        if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap();
        }
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body).unwrap();
    let json = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    (request_line, json)
}

fn write_json_response(stream: &mut TcpStream, body: &Value) {
    let body = body.to_string();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
}

fn spawn_mock_server() -> (
    String,
    Arc<AtomicBool>,
    Arc<Mutex<Vec<Value>>>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let thread_stop = stop.clone();
    let thread_requests = requests.clone();
    let handle = thread::spawn(move || {
        while !thread_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let (request_line, body) = read_http_request(&mut stream);
                    thread_requests.lock().unwrap().push(json!({
                        "request_line": request_line.trim(),
                        "body": body
                    }));
                    if request_line.starts_with("POST /worker/oauth/token ") {
                        write_json_response(
                            &mut stream,
                            &json!({
                                "access_token": mock_access_token(),
                                "refresh_token": "mock-refresh-token",
                                "expires_in": 3600,
                                "refresh_expires_in": 86400
                            }),
                        );
                    } else {
                        write_json_response(&mut stream, &json!({}));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("mock server failed: {error}"),
            }
        }
    });
    (base_url, stop, requests, handle)
}

fn run_command(home: &Path, args: &[&str]) -> CommandOutput {
    let mut command = Command::new(git_ai_binary());
    command.args(args);
    configure_isolated_home(&mut command, home);
    let output = command.output().expect("failed running git-ai command");
    CommandOutput {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

#[test]
fn windows_cli_oauth_login_round_trip() {
    let home = TempDir::new().unwrap();
    let (base_url, stop, requests, server) = spawn_mock_server();
    let stdout_path = home.path().join("login.stdout.log");
    let stderr_path = home.path().join("login.stderr.log");
    let mut command = Command::new(git_ai_binary());
    command
        .args(["login", "--no-browser", "--server", &base_url])
        .stdout(Stdio::from(File::create(&stdout_path).unwrap()))
        .stderr(Stdio::from(File::create(&stderr_path).unwrap()));
    configure_isolated_home(&mut command, home.path());
    let child = command.spawn().expect("failed starting login");

    let authorization_url = wait_for_authorization_url(&stderr_path, Duration::from_secs(10));
    assert_eq!(authorization_url.path(), "/auth/cli/authorize");
    let query = authorization_url
        .query_pairs()
        .into_owned()
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(
        query.get("client_id").map(String::as_str),
        Some("git-ai-cli")
    );
    assert_eq!(query.get("response_type").map(String::as_str), Some("code"));
    assert_eq!(
        query.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert!(
        query
            .get("code_challenge")
            .is_some_and(|value| !value.is_empty())
    );
    let redirect_uri = Url::parse(query.get("redirect_uri").unwrap()).unwrap();
    assert_eq!(redirect_uri.host_str(), Some("127.0.0.1"));
    let state = query.get("state").expect("state should be present");

    send_callback_probe(&redirect_uri);
    send_authorized_callback(&redirect_uri, state);
    let login_status = wait_for_child(child, Duration::from_secs(15));
    assert!(
        login_status.success(),
        "login failed\nstdout:\n{}\nstderr:\n{}",
        read_file(&stdout_path),
        read_file(&stderr_path)
    );

    let token_request = requests
        .lock()
        .unwrap()
        .iter()
        .find(|request| {
            request["request_line"]
                .as_str()
                .unwrap()
                .contains("/worker/oauth/token")
        })
        .cloned()
        .expect("token exchange request should be sent");
    assert_eq!(token_request["body"]["code"], "mock-authorization-code");
    assert_eq!(token_request["body"]["redirect_uri"], redirect_uri.as_str());
    assert!(
        token_request["body"]["code_verifier"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );

    let whoami = run_command(home.path(), &["whoami"]);
    assert!(
        whoami.status.success(),
        "{}{}",
        whoami.stdout,
        whoami.stderr
    );
    assert!(whoami.stdout.contains("Auth state: logged in"));
    assert!(whoami.stdout.contains("User ID: windows-oauth-user"));
    assert!(whoami.stdout.contains("Email: windows@example.test"));

    let logout = run_command(home.path(), &["logout"]);
    assert!(
        logout.status.success(),
        "{}{}",
        logout.stdout,
        logout.stderr
    );
    assert!(logout.stderr.contains("Successfully logged out."));
    let logged_out = run_command(home.path(), &["whoami"]);
    assert!(!logged_out.status.success());
    assert!(logged_out.stdout.contains("Auth state: logged out"));

    let recorded = requests.lock().unwrap();
    let statuses = recorded
        .iter()
        .filter(|request| {
            request["request_line"]
                .as_str()
                .is_some_and(|line| line.contains("/worker/client/status"))
        })
        .filter_map(|request| request["body"]["status"].as_str())
        .collect::<Vec<_>>();
    assert!(statuses.contains(&"logged_in"), "statuses: {statuses:?}");
    assert!(statuses.contains(&"logged_out"), "statuses: {statuses:?}");
    drop(recorded);

    stop.store(true, Ordering::SeqCst);
    server.join().unwrap();
}
