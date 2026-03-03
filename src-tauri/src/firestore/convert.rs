//! Value conversion between Firestore JSON wire format and Rust types.
//!
//! Firestore REST API wraps every value in a tagged object like
//! `{"stringValue": "hello"}` or `{"integerValue": "42"}` (integers are strings!).

use serde_json::{json, Map, Value};

use crate::error::NucleusError;

// ── Read helpers (Firestore → Rust) ─────────────────────────────────────────

pub fn get_string(value: &Value) -> Result<String, NucleusError> {
    value
        .get("stringValue")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| NucleusError::FirestoreQuery("Expected stringValue".into()))
}

pub fn get_bool(value: &Value) -> Result<bool, NucleusError> {
    value
        .get("booleanValue")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| NucleusError::FirestoreQuery("Expected booleanValue".into()))
}

pub fn get_integer(value: &Value) -> Result<i64, NucleusError> {
    // Firestore encodes integers as strings: {"integerValue": "42"}
    // but serde may also decode them as numbers
    let iv = value
        .get("integerValue")
        .ok_or_else(|| NucleusError::FirestoreQuery("Expected integerValue".into()))?;

    if let Some(s) = iv.as_str() {
        s.parse::<i64>()
            .map_err(|_| NucleusError::FirestoreQuery("Invalid integerValue".into()))
    } else if let Some(n) = iv.as_i64() {
        Ok(n)
    } else {
        Err(NucleusError::FirestoreQuery("Invalid integerValue".into()))
    }
}

pub fn get_u32(value: &Value) -> Result<u32, NucleusError> {
    let i = get_integer(value)?;
    u32::try_from(i).map_err(|_| NucleusError::FirestoreQuery("Integer out of u32 range".into()))
}

pub fn get_timestamp(value: &Value) -> Result<String, NucleusError> {
    value
        .get("timestampValue")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| NucleusError::FirestoreQuery("Expected timestampValue".into()))
}

pub fn get_map_fields(value: &Value) -> Result<&Map<String, Value>, NucleusError> {
    value
        .get("mapValue")
        .and_then(|m| m.get("fields"))
        .and_then(|f| f.as_object())
        .ok_or_else(|| NucleusError::FirestoreQuery("Expected mapValue with fields".into()))
}

pub fn get_optional_string(value: &Value) -> Option<String> {
    value.get("stringValue").and_then(|v| v.as_str()).map(|s| s.to_string())
}

pub fn get_optional_timestamp(value: &Value) -> Option<String> {
    value.get("timestampValue").and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// Extract an array of values from a Firestore `arrayValue` field.
/// Returns an empty vec if the field is missing, null, or not an array.
pub fn get_array_values(fields: &Value, key: &str) -> Vec<Value> {
    fields
        .get(key)
        .and_then(|v| v.get("arrayValue"))
        .and_then(|a| a.get("values"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

pub fn get_optional_integer(value: &Value) -> Option<i64> {
    let iv = value.get("integerValue")?;
    if let Some(s) = iv.as_str() {
        s.parse::<i64>().ok()
    } else {
        iv.as_i64()
    }
}

// ── Write helpers (Rust → Firestore) ────────────────────────────────────────

pub fn string_value(s: &str) -> Value {
    json!({"stringValue": s})
}

pub fn boolean_value(b: bool) -> Value {
    json!({"booleanValue": b})
}

pub fn integer_value(i: i64) -> Value {
    json!({"integerValue": i.to_string()})
}

pub fn timestamp_value(ts: &str) -> Value {
    json!({"timestampValue": ts})
}

pub fn null_value() -> Value {
    json!({"nullValue": null})
}

pub fn map_value(fields: serde_json::Map<String, Value>) -> Value {
    json!({"mapValue": {"fields": fields}})
}
