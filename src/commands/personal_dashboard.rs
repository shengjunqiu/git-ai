use crate::config;
use crate::platform::browser::open_url;

/// Handle the `git-ai personal-dashboard` command
pub fn handle_personal_dashboard(_args: &[String]) {
    // Use Config::fresh() to support runtime config updates (daemon mode)
    let config = config::Config::fresh();
    let api_base_url = config.api_base_url();

    let dashboard_url = format!("{}/me", api_base_url);

    eprintln!("Opening dashboard: {}", dashboard_url);

    if open_url(&dashboard_url).is_err() {
        eprintln!("Could not open browser automatically.");
        eprintln!("Visit this URL in your browser:");
        eprintln!("  {}", dashboard_url);
    }
}
