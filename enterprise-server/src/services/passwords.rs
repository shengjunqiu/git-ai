//! Password hashing and verification helpers.

use std::sync::Arc;
use std::time::Instant;

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
    configured_concurrency: usize,
    password: String,
) -> Result<String, AppError> {
    run_password_operation_blocking(
        limiter,
        configured_concurrency,
        "hash",
        "Password hashing",
        move || hash_password(&password),
    )
    .await
}

pub async fn verify_password_blocking(
    limiter: Arc<Semaphore>,
    configured_concurrency: usize,
    password: String,
    password_hash: String,
) -> Result<bool, AppError> {
    run_password_operation_blocking(
        limiter,
        configured_concurrency,
        "verify",
        "Password verification",
        move || verify_password(&password, &password_hash),
    )
    .await
}

async fn run_password_operation_blocking<T, F>(
    limiter: Arc<Semaphore>,
    configured_concurrency: usize,
    operation: &'static str,
    error_context: &'static str,
    operation_fn: F,
) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    let total_started = Instant::now();
    let acquire_started = Instant::now();
    let _permit = limiter
        .acquire_owned()
        .await
        .map_err(|_| AppError::Internal(format!("{error_context} limiter closed")))?;
    let acquire_wait_ms = acquire_started.elapsed().as_secs_f64() * 1000.0;

    let task_result = tokio::task::spawn_blocking(move || {
        let argon_started = Instant::now();
        let result = operation_fn();
        let argon_ms = argon_started.elapsed().as_secs_f64() * 1000.0;
        (result, argon_ms)
    })
    .await;

    let total_ms = total_started.elapsed().as_secs_f64() * 1000.0;
    match task_result {
        Ok((result, argon_ms)) => {
            let operation_result = if result.is_ok() { "ok" } else { "error" };
            tracing::debug!(
                operation,
                configured_concurrency,
                acquire_wait_ms,
                argon_ms,
                total_ms,
                result = operation_result,
                "password operation timing"
            );
            result
        }
        Err(error) => {
            tracing::warn!(
                operation,
                configured_concurrency,
                acquire_wait_ms,
                total_ms,
                error = %error,
                "password operation task failed"
            );
            Err(AppError::Internal(format!(
                "{error_context} task failed: {error}"
            )))
        }
    }
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

        let hash = hash_password_blocking(limiter.clone(), 1, "correct horse".to_string())
            .await
            .unwrap();

        assert!(
            verify_password_blocking(limiter, 1, "correct horse".to_string(), hash)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn blocking_verify_rejects_wrong_password() {
        let limiter = Arc::new(Semaphore::new(1));
        let hash = hash_password("correct horse").unwrap();

        assert!(
            !verify_password_blocking(limiter, 1, "wrong horse".to_string(), hash)
                .await
                .unwrap()
        );
    }
}
