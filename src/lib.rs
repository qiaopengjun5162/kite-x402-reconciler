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

/// Optional local web UI. Compiled only with the `web` feature; the default
/// library and CLI stay free of any network server dependency.
#[cfg(feature = "web")]
pub mod web;

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
        // Replays the first record: same transaction *and* the same nonce for
        // payer `a`. Reusing the nonce (rather than inventing a new one) is
        // what makes the sample exercise both documented findings — the
        // duplicate transaction and the replayed nonce — so `duplicates`
        // actually demonstrates the replay detection it exists for.
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
            nonce: Some("nonce-1".into()),
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
