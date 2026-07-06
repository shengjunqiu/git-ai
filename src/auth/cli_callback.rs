use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

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
                Ok((stream, _addr)) => match handle_stream(stream) {
                    Ok(Some(response)) => return Ok(response),
                    Ok(None) => {}
                    Err(err) => return Err(err),
                },
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

fn handle_stream(mut stream: TcpStream) -> Result<Option<CallbackResponse>, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("Failed to configure callback connection: {}", e))?;

    let request_target = {
        let mut reader = BufReader::new(&mut stream);
        let mut request_line = String::new();
        reader
            .read_line(&mut request_line)
            .map_err(|e| format!("Failed to read callback request: {}", e))?;

        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default();
        let target = parts.next().unwrap_or_default();
        if method != "GET" || target.is_empty() {
            write_html_response(&mut stream, 400, "Invalid authorization callback request.")?;
            return Err("Invalid authorization callback request".to_string());
        }

        let mut header_line = String::new();
        loop {
            header_line.clear();
            let bytes = reader
                .read_line(&mut header_line)
                .map_err(|e| format!("Failed to read callback headers: {}", e))?;
            if bytes == 0 || header_line == "\r\n" || header_line == "\n" {
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
        write_html_response(
            &mut stream,
            200,
            "Authorization was not completed. You can return to your terminal.",
        )?;
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
        write_html_response(
            &mut stream,
            200,
            "Authorization complete. You can return to your terminal.",
        )?;
        CallbackResponse::Authorized { code, state }
    };

    Ok(Some(response))
}

fn write_html_response(stream: &mut TcpStream, status: u16, message: &str) -> Result<(), String> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "OK",
    };
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>git-ai login</title></head><body><p>{}</p></body></html>",
        html_escape(message)
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
        assert!(browser_response.contains("Authorization complete"));
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
        assert!(browser_response.contains("Authorization was not completed"));
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
