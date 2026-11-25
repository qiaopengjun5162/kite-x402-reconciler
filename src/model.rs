//! Core data model for the x402 settlement reconciler.
//!
//! [`SettlementRecord`] mirrors the Kite facilitator's `POST /settle`
//! response (`SettleResponse`) field-for-field, then adds a few optional
//! bookkeeping fields (resource / timestamp / nonce) that the reconciler
//! needs. Keeping the wire fields identical is what makes this tool a real
//! Kite x402 tool rather than a generic ledger.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A CAIP-2 network identifier, e.g. `eip155:2424` (Kite testnet).
///
/// CAIP-2 grammar: `namespace:reference`. We only validate the shape here;
/// we deliberately do not hard-code a chain-id allowlist so the tool keeps
/// working as Kite networks evolve.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NetworkId(String);

impl NetworkId {
    /// Parse a raw CAIP-2 string without validating it.
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Borrow the raw identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Return the namespace half, if the `:` separator is present.
    pub fn namespace(&self) -> Option<&str> {
        self.0.split_once(':').map(|(namespace, _)| namespace)
    }

    /// Return the reference (chain id) half, if present.
    pub fn reference(&self) -> Option<&str> {
        self.0.split_once(':').map(|(_, reference)| reference)
    }

    /// Validate the CAIP-2 shape: non-empty namespace and reference, no
    /// whitespace, exactly one `:`.
    pub fn is_valid(&self) -> bool {
        let Some((namespace, reference)) = self.0.split_once(':') else {
            return false;
        };
        !namespace.is_empty()
            && !reference.is_empty()
            && !namespace.contains(':')
            && !reference.contains(':')
            && !self.0.chars().any(char::is_whitespace)
    }
}

impl fmt::Display for NetworkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One x402 settlement record.
///
/// The first seven fields match the Kite facilitator `SettleResponse`
/// (see `kite-x402-axum/src/types.rs`): the JSON names use the camelCase
/// aliases the facilitator actually returns. The last three are optional
/// metadata the reconciler attaches for auditing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementRecord {
    /// Whether settlement succeeded.
    pub success: bool,

    /// Machine-readable failure reason.
    #[serde(
        rename = "errorReason",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub error_reason: Option<String>,

    /// Human-readable failure explanation.
    #[serde(
        rename = "errorMessage",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub error_message: Option<String>,

    /// Payer wallet address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,

    /// Settlement transaction hash.
    pub transaction: String,

    /// CAIP-2 network identifier.
    pub network: String,

    /// Amount in atomic token units, encoded as a decimal string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,

    /// Resource that was paid for (bookkeeping, optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,

    /// RFC 3339 timestamp of the settlement (bookkeeping, optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,

    /// Payment nonce, for replay detection (bookkeeping, optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
}

/// Canonicalize a hex identifier: trim surrounding whitespace and lowercase.
///
/// This is the single definition of identity normalization in the crate.
/// Transaction hashes and EVM addresses are both case-insensitive hex that
/// arrive from hand-edited exports and mixed-case logs, so every comparison
/// key goes through here and can never disagree about what counts as "the
/// same" value. `None` means "not identifiable" (empty / whitespace only).
fn normalize_hex_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_ascii_lowercase())
    }
}

/// Normalize a settlement transaction hash into the identity key every
/// comparison in this crate must use.
///
/// `0xABCD` and ` 0xabcd ` denote the same on-chain settlement. Both
/// reconciliation pairing and duplicate detection rely on this function so
/// they can never disagree about what counts as the same hash.
#[must_use]
pub fn normalize_hash(raw: &str) -> Option<String> {
    normalize_hex_id(raw)
}

/// Normalize a payer wallet address into the identity key used for replay
/// detection.
///
/// EVM addresses are case-insensitive hex, so `0xABCD` and ` 0xabcd ` denote
/// the same payer. Reusing a nonce under either spelling is one replay, not
/// two independent payments.
#[must_use]
pub fn normalize_address(raw: &str) -> Option<String> {
    normalize_hex_id(raw)
}

/// Normalize a payment nonce into the identity key used for replay detection.
///
/// A nonce arrives from the same hand-edited exports as a hash, so the same
/// rule applies: surrounding whitespace is not part of the value, and an empty
/// nonce is "absent" rather than a value every record shares.
///
/// Case folding is narrower than for a hash. A hex nonce (the EIP-3009
/// `bytes32` form, `0x` + hex digits) has no case, so `0xAB..` and `0xab..`
/// authorize the same transfer. An opaque nonce (`n1`, `order-42`) has no such
/// rule — two spellings may be two different values — so it keeps its case and
/// only gets trimmed.
#[must_use]
pub fn normalize_nonce(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let hex_body = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"));
    match hex_body {
        Some(body) if !body.is_empty() && body.bytes().all(|b| b.is_ascii_hexdigit()) => {
            Some(format!("0x{}", body.to_ascii_lowercase()))
        }
        _ => Some(trimmed.to_owned()),
    }
}

impl SettlementRecord {
    /// View the `network` field as a [`NetworkId`].
    ///
    /// Surrounding whitespace is trimmed: a hand-edited export may pad the
    /// field, and reconciliation compares networks after trimming too (see
    /// `diff::reconcile`). Validation and reconciliation must agree on what
    /// counts as the same network, so the view applies the same trim. Whitespace
    /// *inside* the identifier is preserved and still fails CAIP-2 validation.
    pub fn network_id(&self) -> NetworkId {
        NetworkId::new(self.network.trim().to_owned())
    }

    /// Parse `amount` into an unsigned 128-bit atomic amount.
    ///
    /// `Ok(None)` when the record has no amount; `Ok(Some(value))` on success;
    /// `Err(_)` when the amount is present but not a valid non-negative integer.
    /// A single `Result` keeps callers from peeling two layers of `Option`/`Result`.
    pub fn amount_atomic(&self) -> Result<Option<u128>, std::num::ParseIntError> {
        self.amount
            .as_deref()
            .map(str::trim)
            .map(str::parse::<u128>)
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caip2_valid_forms() {
        assert!(NetworkId::new("eip155:2424").is_valid());
        assert!(NetworkId::new("eip155:1").is_valid());
        assert!(NetworkId::new("kite:testnet").is_valid());
    }

    #[test]
    fn caip2_rejects_malformed() {
        for bad in [
            "",
            "eip155",
            "eip155:",
            ":2424",
            "eip155:2424:extra",
            "eip 155:1",
        ] {
            assert!(!NetworkId::new(bad).is_valid(), "expected invalid: {bad}");
        }
    }

    #[test]
    fn normalize_hash_trims_lowercases_and_rejects_blank() {
        assert_eq!(normalize_hash("  0xABCD  "), Some("0xabcd".to_owned()));
        assert_eq!(normalize_hash("0xabCD"), Some("0xabcd".to_owned()));
        assert_eq!(normalize_hash(""), None);
        assert_eq!(normalize_hash("   "), None);
    }

    #[test]
    fn normalize_address_uses_the_same_rule_as_hash() {
        // A payer address must canonicalize identically to a hash so replay
        // detection and duplicate detection agree on hex identity.
        let address = format!("0x{}", "AB".repeat(20));
        assert_eq!(
            normalize_address(&format!("  {address}  ")),
            normalize_hash(&address)
        );
        assert_eq!(normalize_address("  "), None);
    }

    #[test]
    fn normalize_nonce_trims_and_folds_only_hex_case() {
        assert_eq!(normalize_nonce("  0xABcd  "), Some("0xabcd".to_owned()));
        assert_eq!(normalize_nonce("0XAB"), Some("0xab".to_owned()));
        assert_eq!(normalize_nonce("  n1 "), Some("n1".to_owned()));
        // Opaque nonces have no case rule: folding them would merge two
        // different authorizations into one replay finding.
        assert_eq!(normalize_nonce("AbC"), Some("AbC".to_owned()));
        assert_eq!(normalize_nonce("0xZZ"), Some("0xZZ".to_owned()));
    }

    #[test]
    fn normalize_nonce_treats_blank_as_absent() {
        assert_eq!(normalize_nonce(""), None);
        assert_eq!(normalize_nonce(" \t\n "), None);
    }

    #[test]
    fn caip2_splits_namespace_and_reference() {
        let id = NetworkId::new("eip155:2424");
        assert_eq!(id.namespace(), Some("eip155"));
        assert_eq!(id.reference(), Some("2424"));
        assert_eq!(id.to_string(), "eip155:2424");
    }

    #[test]
    fn settlement_record_deserializes_facilitator_camel_case() {
        let json = r#"{
            "success": true,
            "transaction": "0xabc",
            "network": "eip155:2424",
            "amount": "1000000",
            "payer": "0xpayer"
        }"#;
        let record: SettlementRecord = serde_json::from_str(json).unwrap();
        assert!(record.success);
        assert_eq!(record.network, "eip155:2424");
        assert_eq!(record.amount_atomic(), Ok(Some(1_000_000)));
        assert_eq!(record.resource, None);
    }

    #[test]
    fn settlement_record_round_trips() {
        let record = SettlementRecord {
            success: false,
            error_reason: Some("insufficient_funds".into()),
            error_message: None,
            payer: None,
            transaction: "0xdead".into(),
            network: "eip155:1".into(),
            amount: Some("0".into()),
            resource: Some("/v1/forecast".into()),
            timestamp: Some("2026-11-17T10:00:00Z".into()),
            nonce: Some("n-1".into()),
        };
        let json = serde_json::to_string(&record).unwrap();
        let back: SettlementRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, record);
    }

    #[test]
    fn amount_atomic_handles_missing_and_invalid() {
        let base = SettlementRecord {
            success: true,
            error_reason: None,
            error_message: None,
            payer: None,
            transaction: "0x1".into(),
            network: "eip155:1".into(),
            amount: None,
            resource: None,
            timestamp: None,
            nonce: None,
        };
        assert_eq!(base.amount_atomic(), Ok(None));
        let with_bad = SettlementRecord {
            amount: Some("1.5".into()),
            ..base
        };
        assert!(with_bad.amount_atomic().is_err());
    }

    #[test]
    fn amount_atomic_trims_padding_like_every_other_field() {
        let padded = SettlementRecord {
            amount: Some("  1000  ".into()),
            ..base_record()
        };
        assert_eq!(
            padded.amount_atomic(),
            Ok(Some(1000)),
            "a padded amount must parse, like padded hash/payer/network"
        );
    }

    fn base_record() -> SettlementRecord {
        SettlementRecord {
            success: true,
            error_reason: None,
            error_message: None,
            payer: None,
            transaction: "0x1".into(),
            network: "eip155:1".into(),
            amount: None,
            resource: None,
            timestamp: None,
            nonce: None,
        }
    }
}
