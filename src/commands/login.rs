use crate::auth::{CallbackListener, CallbackResponse, CredentialStore, OAuthClient, pkce};
use crate::config;
use std::time::Duration;
use url::Url;

/// Handle the `git-ai login` command
pub fn handle_login(args: &[String]) {
    let options = match parse_login_options(args) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("Error: {}", e);
            print_help();
            std::process::exit(1);
        }
    };

    if options.help {
        print_help();
        std::process::exit(0);
    }

    let store = CredentialStore::new();

    // Check if already logged in
    if let Ok(Some(creds)) = store.load()
        && !creds.is_refresh_token_expired()
    {
        eprintln!("Already logged in. Use 'git-ai logout' to log out first.");
        std::process::exit(0);
    }

    let client = if let Some(ref url) = options.server_url {
        match OAuthClient::with_base_url(url) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Invalid server URL '{}': {}", url, e);
                std::process::exit(1);
            }
        }
    } else {
        OAuthClient::new()
    };

    let effective_url = client.base_url();

    // Show which server we're connecting to if it's not the default
    let default_url = config::DEFAULT_API_BASE_URL;
    if effective_url != default_url {
        eprintln!("Connecting to server: {}", effective_url);
        eprintln!();
    }

    let completion_redirect_url = format!("{}/me", effective_url.trim_end_matches('/'));
    let listener =
        match CallbackListener::bind_with_completion_redirect(Some(completion_redirect_url)) {
            Ok(listener) => listener,
            Err(e) => {
                eprintln!("Failed to start local callback listener: {}", e);
                std::process::exit(1);
            }
        };
    let redirect_uri = listener.redirect_uri().to_string();
    let state = pkce::generate_state();
    let pkce_pair = pkce::generate_pkce_pair();

    let authorization_url = match build_authorization_url(
        effective_url,
        &redirect_uri,
        &pkce_pair.code_challenge,
        &state,
    ) {
        Ok(url) => url,
        Err(e) => {
            eprintln!("Failed to build authorization URL: {}", e);
            std::process::exit(1);
        }
    };
    let registration_url = match build_registration_url(effective_url, &authorization_url) {
        Ok(url) => url,
        Err(e) => {
            eprintln!("Failed to build registration URL: {}", e);
            std::process::exit(1);
        }
    };

    eprintln!("Starting browser authorization...\n");
    eprintln!("Authorize git-ai in your browser:");
    eprintln!("  {}", authorization_url);
    eprintln!("No account yet? Register first:");
    eprintln!("  {}", registration_url);
    eprintln!();

    if options.no_browser {
        eprintln!(
            "Open the authorization URL above to continue, or open the registration URL first."
        );
    } else if open_browser(&authorization_url).is_err() {
        eprintln!("Could not open browser automatically. Open the URL above to continue.");
        eprintln!();
    }

    eprintln!("Waiting for browser authorization...");

    let callback = match listener.wait_for_callback(Duration::from_secs(300)) {
        Ok(callback) => callback,
        Err(e) => {
            eprintln!("\nAuthorization failed: {}", e);
            std::process::exit(1);
        }
    };

    let code = match callback {
        CallbackResponse::Authorized {
            code,
            state: callback_state,
        } => {
            if let Err(e) = validate_callback_state(&callback_state, &state) {
                eprintln!("\nAuthorization failed: {}", e);
                std::process::exit(1);
            }
            code
        }
        CallbackResponse::Error {
            error,
            state: callback_state,
            error_description,
        } => {
            match callback_state {
                Some(callback_state) => {
                    if let Err(e) = validate_callback_state(&callback_state, &state) {
                        eprintln!("\nAuthorization failed: {}", e);
                        std::process::exit(1);
                    }
                }
                None => {
                    eprintln!("\nAuthorization failed: callback did not include a state");
                    std::process::exit(1);
                }
            }

            let message = authorization_error_message(&error, error_description.as_deref());
            eprintln!("\nAuthorization failed: {}", message);
            std::process::exit(1);
        }
    };

    match client.exchange_authorization_code(&code, &pkce_pair.code_verifier, &redirect_uri) {
        Ok(creds) => {
            if let Err(e) = store.store(&creds) {
                eprintln!("\nWarning: Failed to store credentials: {}", e);
                eprintln!("You may need to log in again next time.");
            }

            // Save the server URL to config if --server was provided
            if let Some(ref url) = options.server_url {
                if let Err(e) = save_server_to_config(url) {
                    eprintln!("\nWarning: Failed to save server URL to config: {}", e);
                    eprintln!(
                        "You may need to set GIT_AI_API_BASE_URL={} in your environment.",
                        url
                    );
                } else {
                    eprintln!("\nServer URL saved to config: {}", url);
                }
            }

            eprintln!("\nSuccessfully logged in!");
        }
        Err(e) => {
            eprintln!("\nToken exchange failed: {}", e);
            std::process::exit(1);
        }
    }
}

/// Attempt to open a URL in the system's default browser
fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut cmd = std::process::Command::new("open");
        cmd.arg(url);
        cmd
    };

    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(url);
        cmd
    };

    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", "start", "", url]);
        cmd
    };

    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct LoginOptions {
    server_url: Option<String>,
    no_browser: bool,
    help: bool,
}

fn parse_login_options(args: &[String]) -> Result<LoginOptions, String> {
    let mut options = LoginOptions::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--server" => {
                i += 1;
                if i >= args.len() {
                    return Err("--server requires a URL".to_string());
                }
                options.server_url = Some(args[i].clone());
            }
            "--no-browser" => {
                options.no_browser = true;
            }
            "--help" | "-h" | "help" => {
                options.help = true;
            }
            arg => {
                if let Some(url) = arg.strip_prefix("--server=") {
                    if url.is_empty() {
                        return Err("--server requires a URL".to_string());
                    }
                    options.server_url = Some(url.to_string());
                } else {
                    return Err(format!("unknown login argument '{}'", arg));
                }
            }
        }
        i += 1;
    }
    Ok(options)
}

fn build_authorization_url(
    base_url: &str,
    redirect_uri: &str,
    code_challenge: &str,
    state: &str,
) -> Result<String, String> {
    let mut url = Url::parse(&format!(
        "{}/auth/cli/authorize",
        base_url.trim_end_matches('/')
    ))
    .map_err(|e| e.to_string())?;

    url.query_pairs_mut()
        .append_pair("client_id", "git-ai-cli")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);

    Ok(url.to_string())
}

fn build_registration_url(base_url: &str, authorization_url: &str) -> Result<String, String> {
    let authorization_url = Url::parse(authorization_url).map_err(|e| e.to_string())?;
    let mut return_to = authorization_url.path().to_string();
    if let Some(query) = authorization_url.query() {
        return_to.push('?');
        return_to.push_str(query);
    }

    let mut url = Url::parse(&format!("{}/auth/register", base_url.trim_end_matches('/')))
        .map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("return_to", &return_to);

    Ok(url.to_string())
}

fn validate_callback_state(callback_state: &str, expected_state: &str) -> Result<(), String> {
    if callback_state == expected_state {
        Ok(())
    } else {
        Err("authorization state did not match".to_string())
    }
}

fn authorization_error_message(error: &str, description: Option<&str>) -> String {
    if error == "access_denied" {
        return "authorization was cancelled".to_string();
    }

    description
        .map(|description| format!("{} ({})", error, description))
        .unwrap_or_else(|| error.to_string())
}

fn print_help() {
    eprintln!("git-ai login - Authenticate with Git AI");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  git-ai login [--server <url>] [--no-browser]");
    eprintln!("  git-ai login --help");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --server <url>   Git AI server URL");
    eprintln!("  --no-browser     Print the authorization URL without opening a browser");
}

/// Save the server URL to the git-ai config file so subsequent commands use it
fn save_server_to_config(url: &str) -> Result<(), String> {
    use std::io::Write;

    let config_dir = dirs::home_dir()
        .ok_or_else(|| "Cannot determine home directory".to_string())?
        .join(".git-ai");

    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config directory: {}", e))?;

    let config_path = config_dir.join("config.json");

    // Read existing config or create new one
    let mut config_json: serde_json::Value = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config: {}", e))?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Set the api_base_url
    config_json["api_base_url"] = serde_json::Value::String(url.to_string());

    // Write back
    let mut file = std::fs::File::create(&config_path)
        .map_err(|e| format!("Failed to create config file: {}", e))?;
    let formatted = serde_json::to_string_pretty(&config_json)
        .map_err(|e| format!("Failed to format config: {}", e))?;
    file.write_all(formatted.as_bytes())
        .map_err(|e| format!("Failed to write config: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parse_login_options_supports_server_space_form() {
        let options = parse_login_options(&strings(&["--server", "https://example.com"])).unwrap();
        assert_eq!(options.server_url.as_deref(), Some("https://example.com"));
        assert!(!options.no_browser);
    }

    #[test]
    fn parse_login_options_supports_server_equals_form() {
        let options = parse_login_options(&strings(&["--server=https://example.com"])).unwrap();
        assert_eq!(options.server_url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn parse_login_options_supports_no_browser() {
        let options = parse_login_options(&strings(&[
            "--server",
            "https://example.com",
            "--no-browser",
        ]))
        .unwrap();
        assert_eq!(options.server_url.as_deref(), Some("https://example.com"));
        assert!(options.no_browser);
    }

    #[test]
    fn parse_login_options_rejects_missing_server_value() {
        let error = parse_login_options(&strings(&["--server"])).unwrap_err();
        assert!(error.contains("requires a URL"));
    }

    #[test]
    fn parse_login_options_supports_help() {
        let options = parse_login_options(&strings(&["--help"])).unwrap();
        assert!(options.help);
    }

    #[test]
    fn parse_login_options_rejects_unknown_argument() {
        let error = parse_login_options(&strings(&["--device-flow"])).unwrap_err();
        assert!(error.contains("unknown login argument"));
    }

    #[test]
    fn authorization_url_contains_expected_parameters() {
        let url = build_authorization_url(
            "https://git-ai.example.com/",
            "http://127.0.0.1:12345/callback",
            "challenge",
            "state",
        )
        .unwrap();
        let parsed = Url::parse(&url).unwrap();
        let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();

        assert_eq!(
            parsed.as_str().split('?').next().unwrap(),
            "https://git-ai.example.com/auth/cli/authorize"
        );
        assert_eq!(
            pairs.get("client_id").map(String::as_str),
            Some("git-ai-cli")
        );
        assert_eq!(
            pairs.get("redirect_uri").map(String::as_str),
            Some("http://127.0.0.1:12345/callback")
        );
        assert_eq!(pairs.get("response_type").map(String::as_str), Some("code"));
        assert_eq!(
            pairs.get("code_challenge").map(String::as_str),
            Some("challenge")
        );
        assert_eq!(
            pairs.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(pairs.get("state").map(String::as_str), Some("state"));
    }

    #[test]
    fn registration_url_preserves_authorization_return_to() {
        let authorization_url = build_authorization_url(
            "https://git-ai.example.com/",
            "http://127.0.0.1:12345/callback",
            "challenge",
            "state",
        )
        .unwrap();
        let url =
            build_registration_url("https://git-ai.example.com/", &authorization_url).unwrap();
        let parsed = Url::parse(&url).unwrap();
        let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();

        assert_eq!(
            parsed.as_str().split('?').next().unwrap(),
            "https://git-ai.example.com/auth/register"
        );
        assert_eq!(
            pairs.get("return_to").map(String::as_str),
            Some(
                "/auth/cli/authorize?client_id=git-ai-cli&redirect_uri=http%3A%2F%2F127.0.0.1%3A12345%2Fcallback&response_type=code&code_challenge=challenge&code_challenge_method=S256&state=state"
            )
        );
    }

    #[test]
    fn callback_state_mismatch_fails() {
        let error = validate_callback_state("actual", "expected").unwrap_err();
        assert!(error.contains("state"));
    }

    #[test]
    fn access_denied_gets_clear_message() {
        assert_eq!(
            authorization_error_message("access_denied", None),
            "authorization was cancelled"
        );
    }

    #[test]
    fn authorization_error_preserves_description() {
        assert_eq!(
            authorization_error_message("invalid_grant", Some("bad verifier")),
            "invalid_grant (bad verifier)"
        );
    }
}
