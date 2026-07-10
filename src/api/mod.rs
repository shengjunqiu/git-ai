//! Public API surface for talking to the remote service.

pub mod bundle;
pub mod cas;
pub mod client;
pub mod client_status;
pub mod metrics;
pub mod types;

// tesrt

// Core client types used by callers across the crate.
pub use client::{ApiClient, ApiContext};
// Helpers for reporting client login state.
pub use client_status::{
    ClientStatusKind, upload_client_status_with_token, upload_current_client_status,
};
// Retry wrapper for metrics uploads.
pub use metrics::upload_metrics_with_retry;
// Shared request and response types.
pub use types::*;
