use super::{
    ControlRequest, ControlResponse, DAEMON_CHECKPOINT_RESPONSE_TIMEOUT,
    DAEMON_CONTROL_CONNECT_TIMEOUT, DAEMON_CONTROL_RESPONSE_TIMEOUT, GitAiError,
};
#[cfg(not(windows))]
use interprocess::{
    ConnectWaitMode,
    local_socket::{ConnectOptions, GenericFilePath, Name, prelude::*},
};
#[cfg(windows)]
use named_pipe::PipeClient as WindowsPipeClient;
#[cfg(windows)]
use std::io::{self, Read};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use tokio::time::Duration;

#[cfg(not(windows))]
pub type DaemonClientStream = LocalSocketStream;

#[cfg(windows)]
pub enum DaemonClientStream {
    WindowsPipe(WindowsPipeClient),
}

#[cfg(windows)]
impl Read for DaemonClientStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::WindowsPipe(stream) => stream.read(buf),
        }
    }
}

#[cfg(windows)]
impl Write for DaemonClientStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::WindowsPipe(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::WindowsPipe(stream) => stream.flush(),
        }
    }
}

pub(super) fn checkpoint_control_timeout_uses_ci_or_test_budget() -> bool {
    std::env::var_os("GIT_AI_TEST_DB_PATH").is_some()
        || std::env::var_os("GITAI_TEST_DB_PATH").is_some()
        || std::env::var_os("CI").is_some()
}

pub(super) fn checkpoint_control_response_timeout(
    request: &ControlRequest,
    use_ci_or_test_budget: bool,
) -> Duration {
    match request {
        // If the caller explicitly asked to wait for checkpoint completion, use
        // the longer budget even in product mode because the request is
        // intentionally synchronous.
        ControlRequest::CheckpointRun { wait, .. } if wait.unwrap_or(false) => {
            DAEMON_CHECKPOINT_RESPONSE_TIMEOUT
        }
        // Queued checkpoint requests can block behind trace-ingest ordering. In
        // CI/test we allow the longer budget so replay-heavy daemon tests don't
        // tear down captured state mid-request. Product mode keeps the short
        // control timeout for fire-and-forget checkpoint requests to preserve
        // responsiveness.
        ControlRequest::CheckpointRun { .. } if use_ci_or_test_budget => {
            DAEMON_CHECKPOINT_RESPONSE_TIMEOUT
        }
        ControlRequest::CheckpointRun { .. } => DAEMON_CONTROL_RESPONSE_TIMEOUT,
        ControlRequest::SnapshotWatermarks { .. } => Duration::from_millis(500),
        _ => DAEMON_CONTROL_RESPONSE_TIMEOUT,
    }
}

fn control_request_response_timeout(request: &ControlRequest) -> Duration {
    checkpoint_control_response_timeout(
        request,
        checkpoint_control_timeout_uses_ci_or_test_budget(),
    )
}

#[cfg(not(windows))]
pub(super) fn local_socket_name<'a>(socket_path: &'a Path) -> Result<Name<'a>, GitAiError> {
    socket_path
        .to_fs_name::<GenericFilePath>()
        .map_err(|e| GitAiError::Generic(format!("invalid daemon socket path: {}", e)))
}

pub fn open_local_socket_stream_with_timeout(
    socket_path: &Path,
    timeout: Duration,
) -> Result<DaemonClientStream, GitAiError> {
    #[cfg(windows)]
    {
        let stream = open_windows_named_pipe_client_with_timeout(socket_path, timeout)?;
        Ok(DaemonClientStream::WindowsPipe(stream))
    }

    #[cfg(not(windows))]
    {
        ConnectOptions::new()
            .name(local_socket_name(socket_path)?)
            .wait_mode(ConnectWaitMode::Timeout(timeout))
            .connect_sync()
            .map_err(|e| {
                GitAiError::Generic(format!(
                    "timed out after {:?} connecting daemon socket {}: {}",
                    timeout,
                    socket_path.display(),
                    e
                ))
            })
    }
}

#[cfg(windows)]
fn open_windows_named_pipe_client_with_timeout(
    socket_path: &Path,
    timeout: Duration,
) -> Result<WindowsPipeClient, GitAiError> {
    let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    WindowsPipeClient::connect_ms(socket_path.as_os_str(), timeout_ms).map_err(|e| {
        GitAiError::Generic(format!(
            "timed out after {:?} connecting daemon socket {}: {}",
            timeout,
            socket_path.display(),
            e
        ))
    })
}

pub(super) fn set_daemon_client_stream_timeouts(
    stream: &mut DaemonClientStream,
    socket_path: &Path,
    timeout: Duration,
) -> Result<(), GitAiError> {
    #[cfg(windows)]
    {
        let _ = socket_path;
        match stream {
            DaemonClientStream::WindowsPipe(pipe) => {
                pipe.set_read_timeout(Some(timeout));
                pipe.set_write_timeout(Some(timeout));
                Ok(())
            }
        }
    }

    #[cfg(not(windows))]
    {
        stream.set_recv_timeout(Some(timeout)).map_err(|e| {
            GitAiError::Generic(format!(
                "failed to set daemon socket {} recv timeout: {}",
                socket_path.display(),
                e
            ))
        })?;
        stream.set_send_timeout(Some(timeout)).map_err(|e| {
            GitAiError::Generic(format!(
                "failed to set daemon socket {} send timeout: {}",
                socket_path.display(),
                e
            ))
        })
    }
}

fn write_all_daemon_client_stream(
    stream: &mut DaemonClientStream,
    socket_path: &Path,
    payload: &[u8],
) -> Result<(), GitAiError> {
    stream.write_all(payload).map_err(|e| {
        GitAiError::Generic(format!(
            "failed writing daemon request to {}: {}",
            socket_path.display(),
            e
        ))
    })?;
    stream.flush().map_err(|e| {
        GitAiError::Generic(format!(
            "failed flushing daemon request to {}: {}",
            socket_path.display(),
            e
        ))
    })?;
    Ok(())
}

fn read_daemon_client_line(
    reader: &mut BufReader<DaemonClientStream>,
    socket_path: &Path,
) -> Result<String, GitAiError> {
    let mut line = String::new();
    let read = reader.read_line(&mut line).map_err(|e| {
        GitAiError::Generic(format!(
            "failed reading daemon response from {}: {}",
            socket_path.display(),
            e
        ))
    })?;
    if read == 0 {
        return Err(GitAiError::Generic(format!(
            "daemon socket {} closed without a response",
            socket_path.display()
        )));
    }
    Ok(line)
}

#[cfg(windows)]
fn send_control_request_with_timeouts_windows(
    socket_path: &Path,
    request: &ControlRequest,
    connect_timeout: Duration,
    response_timeout: Duration,
) -> Result<ControlResponse, GitAiError> {
    let mut stream = open_local_socket_stream_with_timeout(socket_path, connect_timeout)?;
    set_daemon_client_stream_timeouts(&mut stream, socket_path, response_timeout)?;
    let mut body = serde_json::to_vec(request)?;
    body.push(b'\n');
    write_all_daemon_client_stream(&mut stream, socket_path, &body)?;

    let mut response_reader = BufReader::new(stream);
    let line = read_daemon_client_line(&mut response_reader, socket_path)?;
    if line.trim().is_empty() {
        return Err(GitAiError::Generic(
            "empty daemon control response".to_string(),
        ));
    }
    serde_json::from_str(line.trim()).map_err(GitAiError::from)
}

#[cfg(not(windows))]
fn send_control_request_with_timeouts_unix(
    socket_path: &Path,
    request: &ControlRequest,
    connect_timeout: Duration,
    response_timeout: Duration,
) -> Result<ControlResponse, GitAiError> {
    let mut stream = open_local_socket_stream_with_timeout(socket_path, connect_timeout)?;
    set_daemon_client_stream_timeouts(&mut stream, socket_path, response_timeout)?;
    let mut body = serde_json::to_vec(request)?;
    body.push(b'\n');
    write_all_daemon_client_stream(&mut stream, socket_path, &body)?;

    let mut response_reader = BufReader::new(stream);
    let line = read_daemon_client_line(&mut response_reader, socket_path)?;
    if line.trim().is_empty() {
        return Err(GitAiError::Generic(
            "empty daemon control response".to_string(),
        ));
    }
    serde_json::from_str(line.trim()).map_err(GitAiError::from)
}

pub fn local_socket_connects_with_timeout(
    socket_path: &Path,
    timeout: Duration,
) -> Result<(), GitAiError> {
    let _stream = open_local_socket_stream_with_timeout(socket_path, timeout)?;
    Ok(())
}

pub fn send_control_request_with_timeout(
    socket_path: &Path,
    request: &ControlRequest,
    timeout: Duration,
) -> Result<ControlResponse, GitAiError> {
    send_control_request_with_timeouts(socket_path, request, timeout, timeout)
}

fn send_control_request_with_timeouts(
    socket_path: &Path,
    request: &ControlRequest,
    connect_timeout: Duration,
    response_timeout: Duration,
) -> Result<ControlResponse, GitAiError> {
    #[cfg(windows)]
    {
        send_control_request_with_timeouts_windows(
            socket_path,
            request,
            connect_timeout,
            response_timeout,
        )
    }

    #[cfg(not(windows))]
    {
        send_control_request_with_timeouts_unix(
            socket_path,
            request,
            connect_timeout,
            response_timeout,
        )
    }
}

pub fn send_control_request(
    socket_path: &Path,
    request: &ControlRequest,
) -> Result<ControlResponse, GitAiError> {
    send_control_request_with_timeouts(
        socket_path,
        request,
        DAEMON_CONTROL_CONNECT_TIMEOUT,
        control_request_response_timeout(request),
    )
}
