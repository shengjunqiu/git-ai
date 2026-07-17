//! Handle flush-metrics-db command (kept for manual human use).
//!
//! Drains the metrics database queue by uploading batches to the API.

use crate::api::{ApiClient, ApiContext, upload_metrics_with_retry};
use crate::metrics::db::MetricsDatabase;
use crate::metrics::{MetricEvent, MetricsBatch};

/// Max events per batch upload
const MAX_BATCH_SIZE: usize = 250;
const RETRY_DELAY_SECS: u64 = 300;

/// Handle the flush-metrics-db command
pub fn handle_flush_metrics_db(_args: &[String]) {
    // Check conditions: (!using_default_api) || is_logged_in() || has_api_key()
    let context = ApiContext::new(None);
    let api_base_url = context.base_url.clone();
    let client = ApiClient::new(context);

    let using_default_api = api_base_url == crate::config::DEFAULT_API_BASE_URL;
    if using_default_api && !client.is_logged_in() && !client.has_api_key() {
        eprintln!("flush-metrics-db: skipping (not logged in and using default API)");
        return;
    }

    // Get database connection
    let db = match MetricsDatabase::global() {
        Ok(db) => db,
        Err(e) => {
            eprintln!("flush-metrics-db: failed to open metrics database: {}", e);
            return;
        }
    };

    let mut total_uploaded = 0usize;
    let mut total_batches = 0usize;
    let mut total_invalid = 0usize;

    loop {
        // Get batch from DB
        let batch = {
            let db_lock = match db.lock() {
                Ok(lock) => lock,
                Err(e) => {
                    eprintln!("flush-metrics-db: failed to acquire db lock: {}", e);
                    break;
                }
            };
            match db_lock.get_batch(MAX_BATCH_SIZE) {
                Ok(batch) => batch,
                Err(e) => {
                    eprintln!("flush-metrics-db: failed to read batch: {}", e);
                    break;
                }
            }
        };

        // If batch is empty, we're done
        if batch.is_empty() {
            break;
        }

        // Parse events and build MetricsBatch
        let mut events = Vec::new();
        let mut record_ids = Vec::new();

        for record in &batch {
            if let Ok(event) = serde_json::from_str::<MetricEvent>(&record.event_json) {
                events.push(event);
                record_ids.push(record.id);
            } else {
                total_invalid += 1;
                // Invalid JSON - delete the record
                if let Ok(mut db_lock) = db.lock() {
                    let _ = db_lock.delete_records(&[record.id]);
                }
            }
        }

        if events.is_empty() {
            continue;
        }

        let event_count = events.len();
        let metrics_batch = MetricsBatch::new(events);

        // Upload with retry logic (15s, 60s, 3min backoff)
        match upload_metrics_with_retry(&client, &metrics_batch, "flush_metrics_db") {
            Ok(response) => {
                let successful_ids: Vec<i64> = response
                    .successful_indices(event_count)
                    .into_iter()
                    .map(|index| record_ids[index])
                    .collect();
                let failed_ids: Vec<i64> = response
                    .errors
                    .iter()
                    .filter_map(|error| record_ids.get(error.index).copied())
                    .collect();
                total_uploaded += successful_ids.len();
                total_batches += 1;
                eprintln!(
                    "  ✓ batch {} - uploaded {} events",
                    total_batches,
                    successful_ids.len()
                );
                if let Ok(mut db_lock) = db.lock() {
                    let _ = db_lock.delete_records(&successful_ids);
                    if !failed_ids.is_empty() {
                        let failure = response
                            .errors
                            .iter()
                            .map(|error| format!("index {}: {}", error.index, error.error))
                            .collect::<Vec<_>>()
                            .join("; ");
                        let _ =
                            db_lock.record_sync_failure(&failed_ids, &failure, RETRY_DELAY_SECS);
                    }
                }
                if !response.errors.is_empty() {
                    eprintln!(
                        "  ✗ {} event(s) rejected and kept for retry",
                        response.errors.len()
                    );
                    break;
                }
            }
            Err(e) => {
                // All retries failed - keep records in DB for next time
                if let Ok(mut db_lock) = db.lock() {
                    let _ =
                        db_lock.record_sync_failure(&record_ids, &e.to_string(), RETRY_DELAY_SECS);
                }
                eprintln!(
                    "  ✗ batch upload failed ({} events kept for retry): {}",
                    event_count, e
                );
                break;
            }
        }
    }

    if total_invalid > 0 {
        eprintln!(
            "flush-metrics-db: discarded {} invalid record(s)",
            total_invalid
        );
    }

    eprintln!(
        "flush-metrics-db: uploaded {} events in {} batch(es)",
        total_uploaded, total_batches
    );
}
