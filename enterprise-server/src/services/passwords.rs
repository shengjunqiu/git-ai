//! Password hashing and verification helpers.

use std::sync::Arc;

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{password_hash::rand_core::OsRng, Argon2};
use tokio::sync::Semaphore;

use crate::error::AppError;

pub const MIN_PASSWORD_LEN: usize = 8;

pub fn validate_password_strength(password: &str) -> Result<(), AppError> {
    if password.len() < MIN_PASSWORD_LEN {
        return Err(AppError::BadRequest(format!(
            "Password must be at least {} characters",
            MIN_PASSWORD_LEN
        )));
    }

    Ok(())
}

pub fn hash_password(password: &str) -> Result<String, AppError> {
    validate_password_strength(password)?;

    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| AppError::Internal(format!("Failed to hash password: {}", e)))
}

pub fn verify_password(password: &str, password_hash: &str) -> Result<bool, AppError> {
    let parsed_hash = PasswordHash::new(password_hash)
        .map_err(|e| AppError::BadRequest(format!("Invalid password hash: {}", e)))?;

    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

pub async fn hash_password_blocking(
    limiter: Arc<Semaphore>,
    password: String,
) -> Result<String, AppError> {
    let _permit = limiter
        .acquire_owned()
        .await
        .map_err(|_| AppError::Internal("Password hashing limiter closed".into()))?;

    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|error| AppError::Internal(format!("Password hashing task failed: {error}")))?
}

pub async fn verify_password_blocking(
    limiter: Arc<Semaphore>,
    password: String,
    password_hash: String,
) -> Result<bool, AppError> {
    let _permit = limiter
        .acquire_owned()
        .await
        .map_err(|_| AppError::Internal("Password verification limiter closed".into()))?;

    tokio::task::spawn_blocking(move || verify_password(&password, &password_hash))
        .await
        .map_err(|error| {
            AppError::Internal(format!("Password verification task failed: {error}"))
        })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_password_uses_random_salt() {
        let first = hash_password("correct horse").unwrap();
        let second = hash_password("correct horse").unwrap();

        assert_ne!(first, second);
    }

    #[test]
    fn verify_password_accepts_correct_password() {
        let hash = hash_password("correct horse").unwrap();

        assert!(verify_password("correct horse", &hash).unwrap());
    }

    #[test]
    fn verify_password_rejects_wrong_password() {
        let hash = hash_password("correct horse").unwrap();

        assert!(!verify_password("wrong horse", &hash).unwrap());
    }

    #[test]
    fn validate_password_rejects_short_password() {
        assert!(validate_password_strength("short").is_err());
    }

    #[tokio::test]
    async fn blocking_hash_and_verify_accept_correct_password() {
        let limiter = Arc::new(Semaphore::new(1));

        let hash = hash_password_blocking(limiter.clone(), "correct horse".to_string())
            .await
            .unwrap();

        assert!(
            verify_password_blocking(limiter, "correct horse".to_string(), hash)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn blocking_verify_rejects_wrong_password() {
        let limiter = Arc::new(Semaphore::new(1));
        let hash = hash_password("correct horse").unwrap();

        assert!(
            !verify_password_blocking(limiter, "wrong horse".to_string(), hash)
                .await
                .unwrap()
        );
    }
}
