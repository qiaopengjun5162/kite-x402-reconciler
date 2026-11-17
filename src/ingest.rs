//! Ingestion: read settlement records from files/stdin and decode the x402
//! `PAYMENT-RESPONSE` header (base64-encoded JSON).

use std::io::Read;
use std::path::Path;

use base64::Engine;
use serde_json::Value;

use crate::errors::{ReconcileError, Result};
use crate::model::SettlementRecord;

/// Read all settlement records from a path, or from stdin when the path is
/// `-` (or `None`). Accepted encodings, auto-detected in this order:
///
/// 1. A JSON array of records.
/// 2. A single JSON object (a record, or a `PAYMENT-RESPONSE`-style wrapper).
/// 3. JSON Lines (one JSON object per line).
pub fn read_records(path: Option<&Path>) -> Result<Vec<SettlementRecord>> {
    let mut text = String::new();
    match path {
        Some(p) if p.as_os_str() != "-" => {
            let mut file = std::fs::File::open(p)?;
            file.read_to_string(&mut text)?;
        }
        _ => {
            std::io::stdin().read_to_string(&mut text)?;
        }
    }
    parse_records(&text)
}

/// Parse settlement records from raw text. See [`read_records`] for the
/// accepted encodings.
pub fn parse_records(text: &str) -> Result<Vec<SettlementRecord>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    // Try a single JSON value first.
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return match value {
            Value::Array(items) => items
                .into_iter()
                .map(|item| {
                    serde_json::from_value::<SettlementRecord>(item)
                        .map_err(|e| ReconcileError::Decode(format!("invalid record: {e}")))
                })
                .collect(),
            Value::Object(_) => Ok(vec![decode_record_value(&ValueObject(value))?]),
            _ => Err(ReconcileError::Decode(
                "top-level JSON must be an object or array".into(),
            )),
        };
    }

    // Fall back to JSON Lines.
    let mut out = Vec::new();
    for (index, line) in trimmed.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).map_err(|e| {
            ReconcileError::Decode(format!("line {}: invalid JSON: {e}", index + 1))
        })?;
        out.push(decode_record_value(&ValueObject(value))?);
    }
    if out.is_empty() {
        Err(ReconcileError::Decode("no records found in input".into()))
    } else {
        Ok(out)
    }
}

/// Tiny newtype so we can pass a `&Value` that is guaranteed to be an object.
struct ValueObject(Value);

fn decode_record_value(value: &ValueObject) -> Result<SettlementRecord> {
    let object = match &value.0 {
        Value::Object(map) => map,
        _ => {
            return Err(ReconcileError::Decode(
                "record must be a JSON object".into(),
            ))
        }
    };

    // A record may be wrapped in a PAYMENT-RESPONSE-style envelope:
    // {"paymentResponse": "<base64>"} or {"header": "<base64>"}.
    for key in ["paymentResponse", "payment_response", "header"] {
        if let Some(encoded) = object.get(key).and_then(Value::as_str) {
            return decode_payment_response(encoded);
        }
    }

    serde_json::from_value(value.0.clone())
        .map_err(|e| ReconcileError::Decode(format!("invalid record: {e}")))
}

/// Decode a single `PAYMENT-RESPONSE` header value (base64 of JSON) into a
/// settlement record.
pub fn decode_payment_response(header: &str) -> Result<SettlementRecord> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(header.trim())
        .map_err(|e| ReconcileError::Decode(format!("invalid base64: {e}")))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|e| ReconcileError::Decode(format!("decoded bytes are not UTF-8: {e}")))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|e| ReconcileError::Decode(format!("decoded payload is not JSON: {e}")))?;
    decode_record_value(&ValueObject(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_object() -> &'static str {
        r#"{"success":true,"transaction":"0x1","network":"eip155:2424","amount":"100"}"#
    }

    #[test]
    fn parses_json_array() {
        let text = format!("[{}]", sample_object());
        let records = parse_records(&text).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].network, "eip155:2424");
    }

    #[test]
    fn parses_single_object() {
        let records = parse_records(sample_object()).unwrap();
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn parses_json_lines_skipping_blanks() {
        let text = format!("{}\n\n{}\n", sample_object(), sample_object());
        let records = parse_records(&text).unwrap();
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn empty_input_yields_empty_vec() {
        assert!(parse_records("  \n ").unwrap().is_empty());
    }

    #[test]
    fn malformed_input_errors() {
        assert!(parse_records("{not json").is_err());
        assert!(parse_records("42").is_err());
    }

    #[test]
    fn decodes_payment_response_header() {
        let inner = sample_object();
        let encoded = base64::engine::general_purpose::STANDARD.encode(inner);
        let record = decode_payment_response(&encoded).unwrap();
        assert!(record.success);
        assert_eq!(record.amount_atomic(), Ok(Some(100)));
    }

    #[test]
    fn decodes_payment_response_envelope() {
        let inner = sample_object();
        let encoded = base64::engine::general_purpose::STANDARD.encode(inner);
        let text = format!(r#"{{"paymentResponse":"{encoded}"}}"#);
        let records = parse_records(&text).unwrap();
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn rejects_invalid_base64_header() {
        assert!(decode_payment_response("!!!not base64!!!").is_err());
    }

    #[test]
    fn reads_records_from_file() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("kite-reconciler-test-{}.jsonl", std::process::id()));
        std::fs::write(&path, sample_object()).unwrap();
        let records = read_records(Some(&path)).unwrap();
        assert_eq!(records.len(), 1);
        std::fs::remove_file(&path).ok();
    }
}
