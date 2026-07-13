//! CAS secrets detection and sanitization
//!
//! Server-side second-pass verification of CAS content for high-entropy strings
//! that may contain API keys, tokens, or other secrets.
//!
//! The client already performs Shannon entropy-based detection before upload
//! (see src/authorship/secrets.rs), but the server must also verify as a
//! defense-in-depth measure.

/// Minimum length of a string to be considered for entropy checking
const MIN_SECRET_LENGTH: usize = 15;
/// Maximum length of a string to be considered for entropy checking
const MAX_SECRET_LENGTH: usize = 90;
/// Shannon entropy threshold for classifying a string as a potential secret
/// Typical API keys have entropy > 3.5; natural language text is < 3.0
const ENTROPY_THRESHOLD: f64 = 3.5;

/// Result of secrets scanning
#[derive(Debug, Clone)]
pub struct ScanResult {
    /// Number of potential secrets detected
    pub secrets_found: usize,
    /// List of (field_path, detected_value_preview) for audit logging
    pub detections: Vec<(String, String)>,
    /// Whether the content should be blocked
    pub should_block: bool,
}

/// Scan a JSON value for potential secrets using Shannon entropy analysis
pub fn scan_json_for_secrets(value: &serde_json::Value) -> ScanResult {
    let mut detections = Vec::new();
    scan_value_recursive(value, "", &mut detections);

    ScanResult {
        secrets_found: detections.len(),
        should_block: false, // We log but don't block; the client already sanitizes
        detections,
    }
}

fn scan_value_recursive(
    value: &serde_json::Value,
    path: &str,
    detections: &mut Vec<(String, String)>,
) {
    match value {
        serde_json::Value::String(s) => {
            if let Some(secret) = find_high_entropy_substring(s) {
                let preview = secret_preview(&secret);
                detections.push((path.to_string(), preview));
            }
        }
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", path, key)
                };
                scan_value_recursive(val, &child_path, detections);
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, val) in arr.iter().enumerate() {
                let child_path = format!("{}[{}]", path, i);
                scan_value_recursive(val, &child_path, detections);
            }
        }
        _ => {}
    }
}

/// Find a high-entropy substring within a string.
/// Checks candidate tokens delimited by common separators.
fn find_high_entropy_substring(s: &str) -> Option<String> {
    // Split by common delimiters and check each token
    for token in s.split(|c: char| {
        c.is_whitespace()
            || c == '"'
            || c == '\''
            || c == '='
            || c == ':'
            || c == ','
            || c == ';'
            || c == '|'
            || c == '/'
            || c == '\\'
    }) {
        let trimmed = token.trim_matches(|c| {
            c == '"' || c == '\'' || c == '`' || c == '(' || c == ')' || c == '[' || c == ']'
        });
        if trimmed.len() >= MIN_SECRET_LENGTH && trimmed.len() <= MAX_SECRET_LENGTH {
            let entropy = shannon_entropy(trimmed);
            if entropy > ENTROPY_THRESHOLD && looks_like_secret(trimmed) {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Calculate Shannon entropy of a string
pub fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }

    let len = s.len() as f64;
    let mut freq: [usize; 256] = [0; 256];

    for byte in s.bytes() {
        freq[byte as usize] += 1;
    }

    let mut entropy = 0.0;
    for &count in freq.iter() {
        if count > 0 {
            let p = count as f64 / len;
            entropy -= p * p.log2();
        }
    }

    entropy
}

/// Heuristic check: does the string look like a secret?
/// Filters out false positives like URLs, file paths, UUIDs, and common code patterns.
fn looks_like_secret(s: &str) -> bool {
    // Skip URLs
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("ftp://") {
        return false;
    }

    // Skip file paths
    if s.starts_with('/') || s.starts_with("./") || s.starts_with("../") || s.starts_with("C:\\") {
        return false;
    }

    // Skip UUIDs (they have high entropy but are not secrets)
    if is_uuid_like(s) {
        return false;
    }

    // Skip if mostly spaces or repeated characters
    let unique_chars: std::collections::HashSet<char> = s.chars().collect();
    if unique_chars.len() < 4 {
        return false;
    }

    // Skip common code patterns
    if s.contains("function") || s.contains("import ") || s.contains("return ") {
        return false;
    }

    // Skip email-like patterns
    if s.contains('@') && s.contains('.') {
        return false;
    }

    // Likely a secret if it contains mix of uppercase, lowercase, digits, and symbols
    let has_upper = s.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = s.chars().any(|c| c.is_ascii_lowercase());
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    let has_symbol = s.chars().any(|c| !c.is_ascii_alphanumeric());

    // A secret typically has at least 3 of 4 character categories
    let categories = [has_upper, has_lower, has_digit, has_symbol]
        .iter()
        .filter(|&&x| x)
        .count();
    categories >= 3
}

fn secret_preview(secret: &str) -> String {
    let prefix: String = secret.chars().take(4).collect();
    let suffix_chars: Vec<char> = secret.chars().rev().take(4).collect();
    let suffix: String = suffix_chars.into_iter().rev().collect();
    format!("{}...{}", prefix, suffix)
}

/// Check if a string looks like a UUID
fn is_uuid_like(s: &str) -> bool {
    // UUID format: 8-4-4-4-12 hex digits
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 5 {
        let lengths: [usize; 5] = [8, 4, 4, 4, 12];
        return parts.iter().zip(lengths.iter()).all(|(p, &expected_len)| {
            p.len() == expected_len && p.chars().all(|c| c.is_ascii_hexdigit())
        });
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shannon_entropy() {
        // Low entropy (repeated characters)
        assert!(shannon_entropy("aaaaaaa") < 1.0);
        // Medium entropy (natural text)
        assert!(shannon_entropy("hello world") > 2.0 && shannon_entropy("hello world") < 4.0);
        // High entropy (random string)
        assert!(shannon_entropy("aB3$xY9!kL5@mN2#pQ7&") > 3.5);
    }

    #[test]
    fn test_uuid_not_secret() {
        assert!(!looks_like_secret("550e8400-e29b-41d4-a716-446655440000"));
    }

    #[test]
    fn test_url_not_secret() {
        assert!(!looks_like_secret("https://api.example.com/v1/endpoint"));
    }

    #[test]
    fn test_email_not_secret() {
        assert!(!looks_like_secret("user@example.com"));
    }

    #[test]
    fn test_api_key_looks_like_secret() {
        assert!(looks_like_secret("sk-proj-abc123DEF456ghi789JKL012mno345"));
    }

    #[test]
    fn test_scan_json() {
        let json = serde_json::json!({
            "transcript": "User asked about Python",
            "model": "gpt-4",
            "api_key": "sk-proj-abc123DEF456ghi789JKL012mno345PQR678"
        });
        let result = scan_json_for_secrets(&json);
        assert_eq!(result.secrets_found, 1);
        assert!(result.detections[0].0.contains("api_key"));
    }

    #[test]
    fn test_scan_json_handles_unicode_secret_preview() {
        let json = serde_json::json!({
            "message": "GitAiError，不要随意引入新的错误风格。跨平台逻辑用明确的"
        });
        let result = scan_json_for_secrets(&json);
        assert_eq!(result.secrets_found, 1);
        assert_eq!(result.detections[0].1, "GitA...用明确的");
    }
}
