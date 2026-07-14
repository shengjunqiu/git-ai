use std::process::{Command, Stdio};

/// Open a URL with the operating system's default browser.
///
/// Every platform receives the URL as one process argument. In particular,
/// Windows deliberately avoids `cmd /C start`, where `&` in a query string
/// would be interpreted as shell syntax.
pub fn open_url(url: &str) -> Result<(), String> {
    let mut command = browser_command(url);
    command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn browser_command(url: &str) -> Command {
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        command.arg(url);
        command
    }

    #[cfg(target_os = "windows")]
    {
        windows_browser_command(url)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    }
}

#[cfg(any(target_os = "windows", test))]
fn windows_browser_command(url: &str) -> Command {
    let mut command = Command::new("rundll32.exe");
    command.args(["url.dll,FileProtocolHandler", url]);
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn windows_browser_command_passes_url_as_one_argument() {
        let url = "https://example.test/auth?client_id=git-ai-cli&redirect_uri=http%3A%2F%2F127.0.0.1%3A12345%2Fcallback&state=a%26b";
        let command = windows_browser_command(url);

        assert_eq!(command.get_program(), OsStr::new("rundll32.exe"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![OsStr::new("url.dll,FileProtocolHandler"), OsStr::new(url)]
        );
    }

    #[test]
    fn browser_command_keeps_query_ampersands_out_of_shell_parsing() {
        let url = "https://example.test/auth?client_id=git-ai-cli&redirect_uri=http%3A%2F%2F127.0.0.1%3A12345%2Fcallback&state=a%26b";
        let command = browser_command(url);
        let args: Vec<_> = command.get_args().collect();
        assert!(args.iter().any(|arg| *arg == OsStr::new(url)));
        assert!(!args.iter().any(|arg| *arg == OsStr::new("/C")));
    }
}
