//! Password hashing and verification helpers.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Argon2, password_hash::rand_core::OsRng};

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
}
