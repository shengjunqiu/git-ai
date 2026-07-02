use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Releases response matching client's ReleasesResponse
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleasesResponse {
    pub channels: HashMap<String, ChannelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelInfo {
    pub version: String,
    pub checksum: String,             // SHA256 of SHA256SUMS file
}

/// Feature flags response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlagsResponse {
    pub rewrite_stash: bool,
    pub checkpoint_inter_commit_move: bool,
    pub auth_keyring: bool,
    pub async_mode: bool,
    pub git_hooks_enabled: bool,
    pub git_hooks_externally_managed: bool,
}

impl Default for FeatureFlagsResponse {
    fn default() -> Self {
        Self {
            rewrite_stash: true,
            checkpoint_inter_commit_move: false,
            auth_keyring: false,
            async_mode: true,
            git_hooks_enabled: false,
            git_hooks_externally_managed: false,
        }
    }
}
