//! CAS (Content Addressable Storage) service
//!
//! Manages upload and retrieval of AI Prompt records.
//! Content is stored in S3/MinIO, with metadata in PostgreSQL.
//! Hash-based deduplication: same content is stored only once.

use object_store::aws::AmazonS3Builder;
use object_store::path::Path as ObjectPath;
use object_store::ObjectStore;

use crate::config::AppConfig;
use crate::error::AppError;

/// CAS store abstraction backed by S3-compatible object storage
#[derive(Debug, Clone)]
pub struct CasStore {
    store: std::sync::Arc<dyn ObjectStore>,
    bucket: String,
}

impl CasStore {
    pub fn new(config: &AppConfig) -> Result<Self, AppError> {
        let store = AmazonS3Builder::new()
            .with_endpoint(&config.s3_endpoint)
            .with_bucket_name(&config.s3_bucket)
            .with_access_key_id(&config.s3_access_key)
            .with_secret_access_key(&config.s3_secret_key)
            .with_region(&config.s3_region)
            .with_allow_http(true) // For MinIO
            .build()
            .map_err(|e| AppError::CasStorage(format!("Failed to create S3 client: {}", e)))?;

        Ok(Self {
            store: std::sync::Arc::new(store),
            bucket: config.s3_bucket.clone(),
        })
    }

    /// Store CAS content in S3
    /// Key: cas/{first 2 chars of hash}/{hash}
    pub async fn put(&self, hash: &str, content: &[u8]) -> Result<(), AppError> {
        let key = format!("cas/{}/{}", &hash[..2.min(hash.len())], hash);
        let path = ObjectPath::from(key);
        let bytes = bytes::Bytes::copy_from_slice(content);

        self.store
            .put(&path, bytes.into())
            .await
            .map_err(|e| AppError::CasStorage(format!("S3 put failed: {}", e)))?;

        tracing::debug!("CAS put: hash={}, size={} bytes", hash, content.len());
        Ok(())
    }

    /// Retrieve CAS content from S3
    pub async fn get(&self, hash: &str) -> Result<Option<Vec<u8>>, AppError> {
        let key = format!("cas/{}/{}", &hash[..2.min(hash.len())], hash);
        let path = ObjectPath::from(key);

        match self.store.get(&path).await {
            Ok(result) => {
                let bytes = result
                    .bytes()
                    .await
                    .map_err(|e| AppError::CasStorage(format!("S3 get read failed: {}", e)))?;
                tracing::debug!("CAS get: hash={}, size={} bytes", hash, bytes.len());
                Ok(Some(bytes.to_vec()))
            }
            Err(object_store::Error::NotFound { .. }) => {
                tracing::debug!("CAS get: hash={} not found", hash);
                Ok(None)
            }
            Err(e) => Err(AppError::CasStorage(format!("S3 get failed: {}", e))),
        }
    }

    /// Check if content exists in S3
    pub async fn exists(&self, hash: &str) -> Result<bool, AppError> {
        let key = format!("cas/{}/{}", &hash[..2.min(hash.len())], hash);
        let path = ObjectPath::from(key);

        match self.store.head(&path).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(e) => Err(AppError::CasStorage(format!("S3 head failed: {}", e))),
        }
    }

    /// Store release asset in S3
    /// Key: releases/{channel}/{filename}
    pub async fn put_release(&self, channel: &str, filename: &str, content: &[u8]) -> Result<(), AppError> {
        let key = format!("releases/{}/{}", channel, filename);
        let path = ObjectPath::from(key);
        let bytes = bytes::Bytes::copy_from_slice(content);

        self.store
            .put(&path, bytes.into())
            .await
            .map_err(|e| AppError::CasStorage(format!("S3 release put failed: {}", e)))?;

        tracing::info!("Release stored: {}/{} ({} bytes)", channel, filename, content.len());
        Ok(())
    }

    /// Retrieve release asset from S3
    pub async fn get_release(&self, channel: &str, filename: &str) -> Result<Option<Vec<u8>>, AppError> {
        let key = format!("releases/{}/{}", channel, filename);
        let path = ObjectPath::from(key);

        match self.store.get(&path).await {
            Ok(result) => {
                let bytes = result
                    .bytes()
                    .await
                    .map_err(|e| AppError::CasStorage(format!("S3 release get read failed: {}", e)))?;
                Ok(Some(bytes.to_vec()))
            }
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(e) => Err(AppError::CasStorage(format!("S3 release get failed: {}", e))),
        }
    }
}
