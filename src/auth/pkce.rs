use sha2::{Digest, Sha256};
use uuid::Uuid;

pub struct PkcePair {
    pub code_verifier: String,
    pub code_challenge: String,
}

pub fn generate_state() -> String {
    random_hex(2)
}

pub fn generate_pkce_pair() -> PkcePair {
    let code_verifier = generate_code_verifier();
    let code_challenge = code_challenge(&code_verifier);
    PkcePair {
        code_verifier,
        code_challenge,
    }
}

pub fn generate_code_verifier() -> String {
    random_hex(3)
}

pub fn code_challenge(code_verifier: &str) -> String {
    let digest = Sha256::digest(code_verifier.as_bytes());
    base64url_no_pad(&digest)
}

fn random_hex(uuid_count: usize) -> String {
    let mut value = String::with_capacity(uuid_count * 32);
    for _ in 0..uuid_count {
        value.push_str(&Uuid::new_v4().simple().to_string());
    }
    value
}

fn base64url_no_pad(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    let mut encoded = String::with_capacity((bytes.len() * 4).div_ceil(3));
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let value = (b0 << 16) | (b1 << 8) | b2;

        encoded.push(ALPHABET[((value >> 18) & 0x3f) as usize] as char);
        encoded.push(ALPHABET[((value >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(ALPHABET[((value >> 6) & 0x3f) as usize] as char);
        }
        if chunk.len() > 2 {
            encoded.push(ALPHABET[(value & 0x3f) as usize] as char);
        }
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_example() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(code_challenge(verifier), expected);
    }

    #[test]
    fn generated_state_and_verifier_are_unique() {
        let first_state = generate_state();
        let second_state = generate_state();
        let first_verifier = generate_code_verifier();
        let second_verifier = generate_code_verifier();

        assert_ne!(first_state, second_state);
        assert_ne!(first_verifier, second_verifier);
    }

    #[test]
    fn generated_verifier_has_pkce_compatible_length() {
        let verifier = generate_code_verifier();
        assert!(verifier.len() >= 43);
        assert!(verifier.len() <= 128);
        assert!(verifier.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn challenge_has_no_padding() {
        let challenge = code_challenge("test-verifier");
        assert!(!challenge.contains('='));
    }
}
