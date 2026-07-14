use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

const MAX_REQUEST_LINE_BYTES: usize = 8 * 1024;
const MAX_HEADER_LINE_BYTES: usize = 8 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_STREAM_READ_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum CallbackResponse {
    Authorized {
        code: String,
        state: String,
    },
    Error {
        error: String,
        state: Option<String>,
        error_description: Option<String>,
    },
}

pub struct CallbackListener {
    listener: TcpListener,
    redirect_uri: String,
}

impl CallbackListener {
    pub fn bind() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("Failed to bind local callback listener: {}", e))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("Failed to configure callback listener: {}", e))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("Failed to read callback listener address: {}", e))?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{}/callback", port);

        Ok(Self {
            listener,
            redirect_uri,
        })
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    pub fn wait_for_callback(&self, timeout: Duration) -> Result<CallbackResponse, String> {
        let deadline = Instant::now() + timeout;

        loop {
            match self.listener.accept() {
                Ok((stream, _addr)) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err("Timed out waiting for browser authorization".to_string());
                    }
                    match handle_stream(stream, remaining) {
                        Ok(Some(response)) => return Ok(response),
                        Ok(None) => {}
                        // Browsers, endpoint security software, and port checks may
                        // probe a newly opened loopback port before the real OAuth
                        // redirect arrives. A malformed probe must not terminate
                        // the login flow.
                        Err(_) => {}
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err("Timed out waiting for browser authorization".to_string());
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(err) => return Err(format!("Failed to accept callback connection: {}", err)),
            }
        }
    }
}

fn handle_stream(
    mut stream: TcpStream,
    remaining_timeout: Duration,
) -> Result<Option<CallbackResponse>, String> {
    stream
        .set_read_timeout(Some(remaining_timeout.min(MAX_STREAM_READ_TIMEOUT)))
        .map_err(|e| format!("Failed to configure callback connection: {}", e))?;

    let request_target = {
        let mut reader = BufReader::new(&mut stream);
        let request_line =
            read_line_limited(&mut reader, MAX_REQUEST_LINE_BYTES, "callback request")?
                .ok_or_else(|| "Callback connection closed before request".to_string())?;

        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default();
        let target = parts.next().unwrap_or_default();
        if method != "GET" || target.is_empty() {
            write_html_response(&mut stream, 400, "Invalid authorization callback request.")?;
            return Err("Invalid authorization callback request".to_string());
        }

        let mut header_bytes = 0;
        while let Some(header_line) =
            read_line_limited(&mut reader, MAX_HEADER_LINE_BYTES, "callback header")?
        {
            header_bytes += header_line.len();
            if header_bytes > MAX_HEADER_BYTES {
                return Err("Callback request headers exceed the size limit".to_string());
            }
            if header_line == "\r\n" || header_line == "\n" {
                break;
            }
        }

        target.to_string()
    };

    let url = Url::parse(&format!("http://127.0.0.1{}", request_target))
        .map_err(|e| format!("Invalid authorization callback URL: {}", e))?;

    if url.path() != "/callback" {
        write_html_response(&mut stream, 404, "Unknown authorization callback path.")?;
        return Ok(None);
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut error_description = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            "error_description" => error_description = Some(value.into_owned()),
            _ => {}
        }
    }

    let response = if let Some(error) = error {
        write_html_response(&mut stream, 200, "授权未完成。你可以返回终端。")?;
        CallbackResponse::Error {
            error,
            state,
            error_description,
        }
    } else {
        let code =
            code.ok_or_else(|| "Authorization callback did not include a code".to_string())?;
        let state =
            state.ok_or_else(|| "Authorization callback did not include a state".to_string())?;
        write_html_response(&mut stream, 200, "授权成功。你可以返回终端。")?;
        CallbackResponse::Authorized { code, state }
    };

    Ok(Some(response))
}

fn read_line_limited<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
    description: &str,
) -> Result<Option<String>, String> {
    let mut bytes = Vec::new();
    let mut terminated = false;
    loop {
        let buffer = reader
            .fill_buf()
            .map_err(|e| format!("Failed to read {}: {}", description, e))?;
        if buffer.is_empty() {
            break;
        }

        let take = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |position| position + 1);
        if bytes.len() + take > max_bytes {
            return Err(format!("{} exceeds the size limit", description));
        }
        bytes.extend_from_slice(&buffer[..take]);
        reader.consume(take);

        if bytes.last() == Some(&b'\n') {
            terminated = true;
            break;
        }
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    if !terminated && bytes.len() == max_bytes {
        return Err(format!("{} exceeds the size limit", description));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| format!("{} is not valid UTF-8", description))
}

fn write_html_response(stream: &mut TcpStream, status: u16, message: &str) -> Result<(), String> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    };
    let title = if status == 200 {
        "授权成功"
    } else {
        "授权异常"
    };
    let status_class = if status == 200 {
        "status-success"
    } else {
        "status-error"
    };
    let body = format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>git-ai 登录</title>
  <style>{styles}</style>
</head>
<body>
  <main class="auth-shell">
    <section class="auth-card">
      <div class="brand-lockup">
        <div class="brand-title"><span>git-ai</span> Enterprise</div>
        <div class="brand-subtitle">AI 代码归属分析平台</div>
      </div>
      <div class="page-kicker">CLI 授权</div>
      <h1>{title}</h1>
      <div class="status-row {status_class}">
        <span class="status-dot"></span>
        <p>{message}</p>
      </div>
    </section>
  </main>
</body>
</html>"#,
        styles = CALLBACK_PAGE_STYLES,
        title = title,
        status_class = status_class,
        message = html_escape(message),
    );
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        status_text,
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|e| format!("Failed to write callback response: {}", e))
}

const CALLBACK_PAGE_STYLES: &str = r#"
:root {
  font-size: 112.5%;
  --bg-primary: #0f172a;
  --bg-card: #1e293b;
  --border: #334155;
  --text-primary: #f1f5f9;
  --text-secondary: #94a3b8;
  --text-muted: #64748b;
  --accent: #818cf8;
  --success: #34d399;
  --danger: #f87171;
}
* { margin: 0; padding: 0; box-sizing: border-box; }
body {
  margin: 0;
  min-height: 100vh;
  background: var(--bg-primary);
  color: var(--text-primary);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "PingFang SC", "Microsoft YaHei", sans-serif;
}
.auth-shell {
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 2rem 1rem;
}
.auth-card {
  width: min(460px, calc(100vw - 32px));
  border: 1px solid var(--border);
  border-radius: 12px;
  background: var(--bg-card);
  padding: 2rem;
}
.brand-lockup {
  padding-bottom: 1.25rem;
  margin-bottom: 1.5rem;
  border-bottom: 1px solid var(--border);
}
.brand-title {
  font-size: 1.25rem;
  font-weight: 800;
}
.brand-title span {
  color: var(--accent);
}
.brand-subtitle {
  color: var(--text-muted);
  font-size: 0.75rem;
  margin-top: 0.25rem;
}
.page-kicker {
  color: var(--text-muted);
  font-size: 0.7rem;
  text-transform: uppercase;
  letter-spacing: 0.1em;
  margin-bottom: 0.5rem;
}
h1 {
  margin: 0 0 1.5rem;
  font-size: 1.5rem;
  line-height: 1.25;
  font-weight: 700;
}
.status-row {
  display: flex;
  align-items: flex-start;
  gap: 0.75rem;
  padding: 1rem;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--bg-primary);
}
.status-row p {
  margin: 0;
  color: var(--text-secondary);
  line-height: 1.5;
}
.status-dot {
  width: 0.65rem;
  height: 0.65rem;
  border-radius: 999px;
  margin-top: 0.45rem;
  flex: 0 0 auto;
}
.status-success .status-dot {
  background: var(--success);
  box-shadow: 0 0 0 4px rgba(52, 211, 153, 0.12);
}
.status-error .status-dot {
  background: var(--danger);
  box-shadow: 0 0 0 4px rgba(248, 113, 113, 0.12);
}
"#;

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn request_callback(url: &str, target: &str) -> String {
        let parsed = Url::parse(url).unwrap();
        let addr = format!("127.0.0.1:{}", parsed.port().unwrap());
        let mut stream = TcpStream::connect(addr).unwrap();
        write!(
            stream,
            "GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            target
        )
        .unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn callback_receives_code_and_state() {
        let listener = CallbackListener::bind().unwrap();
        let redirect_uri = listener.redirect_uri().to_string();
        let requester = thread::spawn(move || {
            request_callback(&redirect_uri, "/callback?code=abc123&state=state123")
        });

        let response = listener
            .wait_for_callback(Duration::from_secs(2))
            .expect("callback should arrive");

        match response {
            CallbackResponse::Authorized { code, state } => {
                assert_eq!(code, "abc123");
                assert_eq!(state, "state123");
            }
            CallbackResponse::Error { .. } => panic!("expected authorized callback"),
        }

        let browser_response = requester.join().unwrap();
        assert!(browser_response.contains("授权成功"));
        assert!(browser_response.contains("auth-shell"));
        assert!(browser_response.contains("brand-title"));
        assert!(!browser_response.contains("http-equiv=\"refresh\""));
        assert!(!browser_response.contains("/me"));
    }

    #[test]
    fn callback_receives_access_denied_error() {
        let listener = CallbackListener::bind().unwrap();
        let redirect_uri = listener.redirect_uri().to_string();
        let requester = thread::spawn(move || {
            request_callback(
                &redirect_uri,
                "/callback?error=access_denied&state=state123",
            )
        });

        let response = listener
            .wait_for_callback(Duration::from_secs(2))
            .expect("callback should arrive");

        match response {
            CallbackResponse::Error { error, state, .. } => {
                assert_eq!(error, "access_denied");
                assert_eq!(state.as_deref(), Some("state123"));
            }
            CallbackResponse::Authorized { .. } => panic!("expected error callback"),
        }

        let browser_response = requester.join().unwrap();
        assert!(browser_response.contains("授权未完成"));
    }

    #[test]
    fn callback_ignores_empty_probe_before_valid_request() {
        let listener = CallbackListener::bind().unwrap();
        let redirect_uri = listener.redirect_uri().to_string();
        let requester = thread::spawn(move || {
            let parsed = Url::parse(&redirect_uri).unwrap();
            let addr = format!("127.0.0.1:{}", parsed.port().unwrap());
            drop(TcpStream::connect(&addr).unwrap());

            thread::sleep(Duration::from_millis(50));
            request_callback(&redirect_uri, "/callback?code=abc123&state=state123")
        });

        let response = listener
            .wait_for_callback(Duration::from_secs(2))
            .expect("valid callback should arrive after an empty probe");

        match response {
            CallbackResponse::Authorized { code, state } => {
                assert_eq!(code, "abc123");
                assert_eq!(state, "state123");
            }
            CallbackResponse::Error { .. } => panic!("expected authorized callback"),
        }

        assert!(requester.join().unwrap().contains("授权成功"));
    }

    #[test]
    fn callback_ignores_invalid_request_before_valid_request() {
        let listener = CallbackListener::bind().unwrap();
        let redirect_uri = listener.redirect_uri().to_string();
        let requester = thread::spawn(move || {
            let parsed = Url::parse(&redirect_uri).unwrap();
            let addr = format!("127.0.0.1:{}", parsed.port().unwrap());
            let mut invalid = TcpStream::connect(&addr).unwrap();
            write!(
                invalid,
                "POST /callback HTTP/1.1\r\nHost: localhost\r\n\r\n"
            )
            .unwrap();
            drop(invalid);

            thread::sleep(Duration::from_millis(50));
            request_callback(&redirect_uri, "/callback?code=abc123&state=state123")
        });

        let response = listener
            .wait_for_callback(Duration::from_secs(2))
            .expect("valid callback should arrive after an invalid request");
        assert!(matches!(
            response,
            CallbackResponse::Authorized { code, state }
                if code == "abc123" && state == "state123"
        ));
        assert!(requester.join().unwrap().contains("授权成功"));
    }

    #[test]
    fn callback_ignores_oversized_request_before_valid_request() {
        let listener = CallbackListener::bind().unwrap();
        let redirect_uri = listener.redirect_uri().to_string();
        let requester = thread::spawn(move || {
            let parsed = Url::parse(&redirect_uri).unwrap();
            let addr = format!("127.0.0.1:{}", parsed.port().unwrap());
            let mut oversized = TcpStream::connect(&addr).unwrap();
            let request = format!(
                "GET /callback?{} HTTP/1.1\r\nHost: localhost\r\n\r\n",
                "x".repeat(MAX_REQUEST_LINE_BYTES)
            );
            oversized.write_all(request.as_bytes()).unwrap();
            drop(oversized);

            thread::sleep(Duration::from_millis(50));
            request_callback(&redirect_uri, "/callback?code=abc123&state=state123")
        });

        let response = listener
            .wait_for_callback(Duration::from_secs(2))
            .expect("valid callback should arrive after an oversized request");
        assert!(matches!(response, CallbackResponse::Authorized { .. }));
        assert!(requester.join().unwrap().contains("授权成功"));
    }

    #[test]
    fn invalid_probes_do_not_extend_total_timeout() {
        let listener = CallbackListener::bind().unwrap();
        let redirect_uri = listener.redirect_uri().to_string();
        let requester = thread::spawn(move || {
            let parsed = Url::parse(&redirect_uri).unwrap();
            let addr = format!("127.0.0.1:{}", parsed.port().unwrap());
            let end = Instant::now() + Duration::from_millis(500);
            while Instant::now() < end {
                if let Ok(mut stream) = TcpStream::connect(&addr) {
                    let _ = stream.write_all(b"POST /callback HTTP/1.1\r\n\r\n");
                }
                thread::sleep(Duration::from_millis(10));
            }
        });

        let started = Instant::now();
        let error = listener
            .wait_for_callback(Duration::from_millis(120))
            .unwrap_err();
        let elapsed = started.elapsed();
        assert!(error.contains("Timed out"));
        assert!(elapsed < Duration::from_millis(500), "elapsed: {elapsed:?}");
        requester.join().unwrap();
    }

    #[test]
    fn callback_times_out() {
        let listener = CallbackListener::bind().unwrap();
        let error = listener
            .wait_for_callback(Duration::from_millis(20))
            .unwrap_err();
        assert!(error.contains("Timed out"));
    }
}
