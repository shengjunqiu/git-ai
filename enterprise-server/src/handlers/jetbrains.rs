//! JetBrains plugin download proxy
//!
//! Proxies plugin download requests to the JetBrains Marketplace,
//! allowing enterprise users to download plugins without direct internet access.

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::AppState;

#[derive(Debug, Deserialize)]
pub struct PluginDownloadQuery {
    /// Plugin ID (e.g., "com.usegitai.intellij")
    pub id: Option<String>,
    /// IDE build number (e.g., "IU-241.15989")
    pub build: Option<String>,
}

/// GET /worker/plugins/jetbrains/download — Proxy JetBrains plugin download
///
/// Proxies the request to JetBrains Marketplace and returns the plugin ZIP.
/// Can be pre-cached for common IDE versions.
pub async fn download_jetbrains_plugin(
    State(_state): State<AppState>,
    Query(query): Query<PluginDownloadQuery>,
) -> Result<Response, AppError> {
    let plugin_id = query.id.as_deref().unwrap_or("com.usegitai.intellij");
    let build = query.build.as_deref().unwrap_or("");

    if build.is_empty() {
        return Err(AppError::BadRequest("build parameter is required".into()));
    }

    let url = format!(
        "https://plugins.jetbrains.com/pluginManager/action=download&id={}&build={}",
        plugin_id, build
    );

    tracing::info!("JetBrains plugin proxy: id={}, build={}", plugin_id, build);

    // Fetch from JetBrains Marketplace
    let client = reqwest::Client::new();
    let response = client.get(&url)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to fetch plugin: {}", e)))?;

    if !response.status().is_success() {
        return Err(AppError::NotFound(format!(
            "Plugin not found on JetBrains Marketplace (status: {})",
            response.status()
        )));
    }

    let content_type = response.headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/zip")
        .to_string();

    let content_length = response.headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());

    let data = response.bytes().await
        .map_err(|e| AppError::Internal(format!("Failed to read plugin data: {}", e)))?;

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, &content_type)
        .header(header::CACHE_CONTROL, "public, max-age=3600");

    if let Some(len) = content_length {
        builder = builder.header(header::CONTENT_LENGTH, len);
    }

    Ok(builder.body(Body::from(data.to_vec()))
        .map_err(|e| AppError::Internal(format!("Failed to build response: {}", e)))?)
}
