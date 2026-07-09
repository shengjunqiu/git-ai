#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;

pub const DEFAULT_LIMIT: i64 = 100;
pub const MAX_LIMIT: i64 = 1000;
pub const DASHBOARD_MAX_LIMIT: i64 = 200;
pub const CURSOR_VERSION: u8 = 1;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PaginationQuery {
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeIdCursor {
    pub v: u8,
    pub timestamp: DateTime<Utc>,
    pub id: i64,
}

impl TimeIdCursor {
    pub fn new(timestamp: DateTime<Utc>, id: i64) -> Self {
        Self {
            v: CURSOR_VERSION,
            timestamp,
            id,
        }
    }

    fn validate(&self) -> Result<(), AppError> {
        validate_cursor_version(self.v)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeUuidCursor {
    pub v: u8,
    pub timestamp: DateTime<Utc>,
    pub id: Uuid,
}

impl TimeUuidCursor {
    pub fn new(timestamp: DateTime<Utc>, id: Uuid) -> Self {
        Self {
            v: CURSOR_VERSION,
            timestamp,
            id,
        }
    }

    fn validate(&self) -> Result<(), AppError> {
        validate_cursor_version(self.v)
    }
}

pub fn clamp_limit(input: Option<i64>, default: i64, max: i64) -> i64 {
    let max = max.max(1);
    let default = default.clamp(1, max);
    input.unwrap_or(default).clamp(1, max)
}

pub fn fetch_limit(limit: i64) -> i64 {
    limit.saturating_add(1)
}

pub fn truncate_to_limit<T>(items: &mut Vec<T>, limit: i64) -> bool {
    let limit = limit.max(0) as usize;
    if items.len() > limit {
        items.truncate(limit);
        true
    } else {
        false
    }
}

pub fn pagination_meta(limit: i64, has_more: bool, next_cursor: Option<String>) -> Value {
    json!({
        "limit": limit,
        "has_more": has_more,
        "next_cursor": next_cursor,
    })
}

pub fn encode_cursor<T: Serialize>(cursor: &T) -> Result<String, AppError> {
    let bytes = serde_json::to_vec(cursor)
        .map_err(|e| AppError::Internal(format!("Failed to encode pagination cursor: {}", e)))?;
    Ok(base64url_encode(&bytes))
}

pub fn decode_cursor<T: DeserializeOwned>(cursor: &str) -> Result<T, AppError> {
    let bytes = base64url_decode(cursor)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| AppError::BadRequest("Invalid pagination cursor".into()))
}

pub fn decode_time_id_cursor(cursor: &str) -> Result<TimeIdCursor, AppError> {
    let cursor: TimeIdCursor = decode_cursor(cursor)?;
    cursor.validate()?;
    Ok(cursor)
}

pub fn decode_time_uuid_cursor(cursor: &str) -> Result<TimeUuidCursor, AppError> {
    let cursor: TimeUuidCursor = decode_cursor(cursor)?;
    cursor.validate()?;
    Ok(cursor)
}

fn validate_cursor_version(version: u8) -> Result<(), AppError> {
    if version == CURSOR_VERSION {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "Unsupported pagination cursor version: {}",
            version
        )))
    }
}

fn base64url_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    let mut out = String::with_capacity((bytes.len() * 4).div_ceil(3));
    let mut chunks = bytes.chunks_exact(3);

    for chunk in &mut chunks {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        out.push(TABLE[(n & 0x3f) as usize] as char);
    }

    match chunks.remainder() {
        [a] => {
            out.push(TABLE[(a >> 2) as usize] as char);
            out.push(TABLE[((a & 0x03) << 4) as usize] as char);
        }
        [a, b] => {
            out.push(TABLE[(a >> 2) as usize] as char);
            out.push(TABLE[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);
            out.push(TABLE[((b & 0x0f) << 2) as usize] as char);
        }
        [] => {}
        _ => unreachable!(),
    }

    out
}

fn base64url_decode(input: &str) -> Result<Vec<u8>, AppError> {
    if input.is_empty() || input.len() % 4 == 1 {
        return Err(AppError::BadRequest("Invalid pagination cursor".into()));
    }

    let mut out = Vec::with_capacity((input.len() * 3) / 4);
    let mut buffer = 0u32;
    let mut bits = 0u8;

    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return Err(AppError::BadRequest("Invalid pagination cursor".into())),
        };

        buffer = (buffer << 6) | u32::from(value);
        bits += 6;

        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
            buffer &= if bits == 0 { 0 } else { (1u32 << bits) - 1 };
        }
    }

    if bits > 0 && buffer != 0 {
        return Err(AppError::BadRequest("Invalid pagination cursor".into()));
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn clamp_limit_defaults_to_default_value() {
        assert_eq!(clamp_limit(None, 100, 1000), 100);
    }

    #[test]
    fn clamp_limit_raises_zero_to_one() {
        assert_eq!(clamp_limit(Some(0), 100, 1000), 1);
    }

    #[test]
    fn clamp_limit_caps_large_values() {
        assert_eq!(clamp_limit(Some(5000), 100, 1000), 1000);
    }

    #[test]
    fn base64url_encoder_omits_padding() {
        assert_eq!(base64url_encode(b"f"), "Zg");
        assert_eq!(base64url_encode(b"fo"), "Zm8");
        assert_eq!(base64url_encode(b"foo"), "Zm9v");
    }

    #[test]
    fn time_id_cursor_round_trips() {
        let timestamp = Utc.with_ymd_and_hms(2026, 7, 9, 10, 0, 0).unwrap();
        let cursor = TimeIdCursor::new(timestamp, 42);

        let encoded = encode_cursor(&cursor).unwrap();
        let decoded = decode_time_id_cursor(&encoded).unwrap();

        assert_eq!(decoded, cursor);
    }

    #[test]
    fn time_uuid_cursor_round_trips() {
        let timestamp = Utc.with_ymd_and_hms(2026, 7, 9, 10, 0, 0).unwrap();
        let cursor = TimeUuidCursor::new(timestamp, Uuid::new_v4());

        let encoded = encode_cursor(&cursor).unwrap();
        let decoded = decode_time_uuid_cursor(&encoded).unwrap();

        assert_eq!(decoded, cursor);
    }

    #[test]
    fn invalid_cursor_returns_bad_request() {
        let err = decode_time_id_cursor("not*base64url").unwrap_err();

        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn unsupported_cursor_version_returns_bad_request() {
        let timestamp = Utc.with_ymd_and_hms(2026, 7, 9, 10, 0, 0).unwrap();
        let encoded = encode_cursor(&json!({
            "v": 2,
            "timestamp": timestamp,
            "id": 42
        }))
        .unwrap();

        let err = decode_time_id_cursor(&encoded).unwrap_err();

        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn truncate_to_limit_reports_has_more() {
        let mut items = vec![1, 2, 3];

        let has_more = truncate_to_limit(&mut items, 2);

        assert!(has_more);
        assert_eq!(items, vec![1, 2]);
    }
}
