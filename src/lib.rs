//! `kite-x402-reconciler` — reconciliation and audit toolkit for Kite x402
//! settlement records.
//!
//! The data model mirrors the Kite facilitator's `SettleResponse`, so this
//! crate works with real x402 settlement output: JSON/JSONL ledgers and
//! base64-encoded `PAYMENT-RESPONSE` headers.

pub mod diff;
pub mod errors;
pub mod gate;
pub mod ingest;
pub mod model;
pub mod reconcile;
pub mod report;
pub mod validate;

use model::SettlementRecord;

/// Build a small sample ledger for demos and tests.
///
/// Includes one duplicate transaction and one replayed nonce so that
/// `validate` has something interesting to report.
pub fn sample_records() -> Vec<SettlementRecord> {
    let tx = |seed: char| format!("0x{}", seed.to_string().repeat(64));
    let payer = |seed: char| format!("0x{}", seed.to_string().repeat(40));

    vec![
        SettlementRecord {
            success: true,
            error_reason: None,
            error_message: None,
            payer: Some(payer('a')),
            transaction: tx('1'),
            network: "eip155:2424".into(),
            amount: Some("1500000".into()),
            resource: Some("/v1/forecast".into()),
            timestamp: Some("2026-11-17T09:15:00Z".into()),
            nonce: Some("nonce-1".into()),
        },
        SettlementRecord {
            success: true,
            error_reason: None,
            error_message: None,
            payer: Some(payer('b')),
            transaction: tx('2'),
            network: "eip155:2424".into(),
            amount: Some("500000".into()),
            resource: Some("/v1/weather".into()),
            timestamp: Some("2026-11-17T10:40:00Z".into()),
            nonce: Some("nonce-2".into()),
        },
        // Duplicate transaction of the first record.
        SettlementRecord {
            success: true,
            error_reason: None,
            error_message: None,
            payer: Some(payer('a')),
            transaction: tx('1'),
            network: "eip155:2424".into(),
            amount: Some("1500000".into()),
            resource: Some("/v1/forecast".into()),
            timestamp: Some("2026-11-17T11:00:00Z".into()),
            nonce: Some("nonce-3".into()),
        },
        // Failed settlement on a different network.
        SettlementRecord {
            success: false,
            error_reason: Some("insufficient_funds".into()),
            error_message: None,
            payer: Some(payer('c')),
            transaction: tx('3'),
            network: "eip155:1".into(),
            amount: None,
            resource: Some("/v1/forecast".into()),
            timestamp: Some("2026-11-18T08:00:00Z".into()),
            nonce: Some("nonce-4".into()),
        },
    ]
}
