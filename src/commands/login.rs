use crate::auth::{CredentialStore, OAuthClient};
use crate::config;

/// Handle the `git-ai login` command
pub fn handle_login(args: &[String]) {
    // Parse --server <url> from args
    let server_url = parse_server_arg(args);

    let store = CredentialStore::new();

    // Check if already logged in
    if let Ok(Some(creds)) = store.load()
        && !creds.is_refresh_token_expired()
    {
        eprintln!("Already logged in. Use 'git-ai logout' to log out first.");
        std::process::exit(0);
    }

    let client = if let Some(ref url) = server_url {
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

    // Start device flow
    eprintln!("Starting device authorization...\n");

    let auth_response = match client.start_device_flow() {
        Ok(response) => response,
        Err(e) => {
            eprintln!("Failed to start authorization: {}", e);
            std::process::exit(1);
        }
    };

    // Build the display URL
    let display_url = auth_response
        .verification_uri_complete
        .as_ref()
        .unwrap_or(&auth_response.verification_uri);

    // Display instructions
    eprintln!("To authorize this device:");
    eprintln!("  1. Open this URL in your browser:");
    eprintln!("     {}", display_url);
    eprintln!();
    eprintln!("  2. Enter this code when prompted:");
    eprintln!("     {}", auth_response.user_code);
    eprintln!();

    // Try to open browser automatically
    if open_browser(display_url).is_err() {
        eprintln!("  (Could not open browser automatically)");
        eprintln!();
    }

    eprintln!("Waiting for authorization...");

    // Poll for token
    match client.poll_for_token(
        &auth_response.device_code,
        auth_response.interval,
        auth_response.expires_in,
    ) {
        Ok(creds) => {
            // Store credentials
            if let Err(e) = store.store(&creds) {
                eprintln!("\nWarning: Failed to store credentials: {}", e);
                eprintln!("You may need to log in again next time.");
            }

            // Save the server URL to config if --server was provided
            if let Some(ref url) = server_url {
                if let Err(e) = save_server_to_config(url) {
                    eprintln!("\nWarning: Failed to save server URL to config: {}", e);
                    eprintln!("You may need to set GIT_AI_API_BASE_URL={} in your environment.", url);
                } else {
                    eprintln!("\nServer URL saved to config: {}", url);
                }
            }

            eprintln!("\nSuccessfully logged in!");
        }
        Err(e) => {
            eprintln!("\nAuthorization failed: {}", e);
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

/// Parse --server <url> from args
fn parse_server_arg(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--server" && i + 1 < args.len() {
            return Some(args[i + 1].clone());
        }
        if let Some(url) = args[i].strip_prefix("--server=") {
            return Some(url.to_string());
        }
        i += 1;
    }
    None
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
