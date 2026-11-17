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

impl SettlementRecord {
    /// View the `network` field as a [`NetworkId`].
    pub fn network_id(&self) -> NetworkId {
        NetworkId::new(self.network.clone())
    }

    /// Parse `amount` into an unsigned 128-bit atomic amount.
    ///
    /// Returns `None` when the record has no amount. Returns `Some(Err(_))`
    /// when the amount string is present but not a valid non-negative integer.
    pub fn amount_atomic(&self) -> Option<Result<u128, std::num::ParseIntError>> {
        self.amount.as_deref().map(|raw| raw.parse::<u128>())
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
        assert_eq!(record.amount_atomic(), Some(Ok(1_000_000)));
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
        assert_eq!(base.amount_atomic(), None);
        let with_bad = SettlementRecord {
            amount: Some("1.5".into()),
            ..base
        };
        assert!(with_bad.amount_atomic().unwrap().is_err());
    }
}
