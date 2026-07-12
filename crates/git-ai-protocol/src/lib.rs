//! Transport-only request and response types shared by git-ai clients and servers.
//!
//! Keep domain behavior and persistence models out of this crate. Types here define
//! the serialized JSON contract at the network boundary.

pub mod api;
pub mod bundle;
pub mod cas;
pub mod client_status;
pub mod metrics;
pub mod oauth;
pub mod release;
pub mod report;
