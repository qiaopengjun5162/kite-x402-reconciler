//! Two-ledger reconciliation: compare the settlements your service recorded
//! against the settlements the facilitator reports.
//!
//! Every other command in this crate audits **one** ledger. That answers "is
//! this ledger self-consistent", not "does it agree with the chain". Those are
//! different questions, and only the second one catches the failures that cost
//! money:
//!
//! - **Local only** — your service claims a settlement the facilitator never
//!   made. If you bill or deliver off your own ledger, you are invoicing
//!   revenue that does not exist.
//! - **Facilitator only** — the buyer's money moved and your service has no
//!   record of the request. The buyer paid for something you may never have
//!   delivered, and a retry will charge them twice.
//! - **Amount mismatch** — both sides saw the same transaction but disagree
//!   on how much it was worth.
//! - **Network mismatch** — the same hash is recorded against a different
//!   CAIP-2 chain. The payment landed somewhere other than your ledger says,
//!   so billing and delivery driven off that record point at the wrong chain.
//!
//! # Matching
//!
//! Records are matched on the settlement transaction hash, which is the only
//! identifier both sides are guaranteed to share. It is normalized by trimming
//! whitespace and lowercasing, because hex hashes reach us from hand-edited
//! exports and mixed-case logs. The CAIP-2 network must then agree before the
//! pair can be called a match; network identifiers are trimmed but **not**
//! case-folded, since CAIP-2 references may be case-sensitive.
//!
//! Failed settlements are deliberately excluded from matching. A failed
//! `SettleResponse` has no on-chain transaction, so pairing one against the
//! other side's silence would report every failure as a discrepancy twice
//! over. They are counted in [`LedgerDiff::skipped_failed`] instead, so the
//! operator can see how much of the ledger was out of scope.
//!
//! A *successful* record with an empty transaction hash is different: it is a
//! settlement that claims to have happened but cannot be identified. That is a
//! data defect, so it is surfaced as a warning rather than quietly skipped.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::model::{normalize_hash, SettlementRecord};

/// A settlement that exists on only one side of the comparison.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiffEntry {
    /// Transaction hash as it appeared in that ledger (trimmed).
    pub transaction: String,
    /// CAIP-2 network identifier.
    pub network: String,
    /// Atomic amount, when the record carried one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    /// Payer address, when the record carried one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,
}

/// Two records sharing a transaction hash but disagreeing on the amount.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AmountMismatch {
    /// Transaction hash (taken from the local record, trimmed).
    pub transaction: String,
    /// Atomic amount the local ledger recorded.
    pub local_amount: Option<String>,
    /// Atomic amount the facilitator reported.
    pub facilitator_amount: Option<String>,
}

/// Two records sharing a transaction hash but disagreeing on the CAIP-2
/// network the settlement happened on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NetworkMismatch {
    /// Transaction hash (taken from the local record, trimmed).
    pub transaction: String,
    /// Network identifier the local ledger recorded.
    pub local_network: String,
    /// Network identifier the facilitator reported.
    pub facilitator_network: String,
}

/// Result of comparing a local ledger against a facilitator ledger.
#[derive(Clone, Debug, Default, Serialize)]
pub struct LedgerDiff {
    /// Records read from the local ledger.
    pub local_records: usize,
    /// Records read from the facilitator ledger.
    pub facilitator_records: usize,
    /// Transaction pairs actually compared (one local + one facilitator each).
    pub compared: usize,
    /// Pairs whose amounts agree.
    pub matched: usize,
    /// Pairs whose amounts disagree.
    pub amount_mismatch: Vec<AmountMismatch>,
    /// Pairs recorded against different CAIP-2 networks.
    pub network_mismatch: Vec<NetworkMismatch>,
    /// Settlements only the local ledger has.
    pub only_local: Vec<DiffEntry>,
    /// Settlements only the facilitator ledger has.
    pub only_facilitator: Vec<DiffEntry>,
    /// Records skipped because `success` was false — no on-chain settlement
    /// exists to reconcile, so these are expected, not findings.
    pub skipped_failed: usize,
    /// Successful records skipped because they carried no transaction hash.
    pub skipped_unidentified: usize,
}

impl LedgerDiff {
    /// Number of discrepancies: every pair that disagreed and every
    /// one-sided settlement. These are the findings a gate should fail on.
    #[must_use]
    pub fn errors(&self) -> usize {
        self.amount_mismatch.len()
            + self.network_mismatch.len()
            + self.only_local.len()
            + self.only_facilitator.len()
    }

    /// Number of soft findings: claims of settlement that cannot be
    /// identified, so they silently fell out of the comparison.
    #[must_use]
    pub fn warnings(&self) -> usize {
        self.skipped_unidentified
    }

    /// True when the two ledgers agree and nothing was left unidentified.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.errors() == 0 && self.warnings() == 0
    }
}

/// Normalize a transaction hash into the key both sides are matched on.
///
/// `None` means "not identifiable" — an empty or whitespace-only hash. This is
/// what separates a failed settlement (expected, counted) from a successful
/// one with no hash (a defect, warned about).
fn match_key(record: &SettlementRecord) -> Option<String> {
    normalize_hash(&record.transaction)
}

/// Successful records bucketed by transaction key, plus the skip counters.
#[derive(Default)]
struct Side<'a> {
    by_tx: BTreeMap<String, Vec<&'a SettlementRecord>>,
    skipped_failed: usize,
    skipped_unidentified: usize,
}

impl<'a> Side<'a> {
    fn build(records: &'a [SettlementRecord]) -> Self {
        let mut side = Self::default();
        for record in records {
            // A failed settlement never reached the chain, so there is nothing
            // to pair it against. Counting it as a discrepancy on both sides
            // would double-report every failure.
            if !record.success {
                side.skipped_failed += 1;
                continue;
            }
            match match_key(record) {
                Some(key) => side.by_tx.entry(key).or_default().push(record),
                None => side.skipped_unidentified += 1,
            }
        }
        side
    }
}

fn entry(record: &SettlementRecord) -> DiffEntry {
    DiffEntry {
        transaction: record.transaction.trim().to_owned(),
        network: record.network.clone(),
        amount: record.amount.clone(),
        payer: record.payer.clone(),
    }
}

/// Decide whether two amounts describe the same value.
///
/// Comparing the raw strings would flag `"1000000"` against `"01000000"`, which
/// is the same atomic amount written differently. Comparing parsed values is
/// the right test, but it is only available when both sides parse; when either
/// is missing or malformed we fall back to exact string equality so that
/// "absent" stays equal to "absent" and a malformed value never silently
/// matches a valid one. Both sides are trimmed first, matching the rest of the
/// identity pipeline: a hand-edited export with a padded amount is the same
/// value, not a mismatch.
fn same_amount(local: &Option<String>, facilitator: &Option<String>) -> bool {
    match (
        local.as_deref().map(str::trim).map(str::parse::<u128>),
        facilitator
            .as_deref()
            .map(str::trim)
            .map(str::parse::<u128>),
    ) {
        (Some(Ok(left)), Some(Ok(right))) => left == right,
        _ => local.as_deref().map(str::trim) == facilitator.as_deref().map(str::trim),
    }
}

/// Compare a local ledger against the facilitator's ledger.
///
/// Ordering of the output follows the records' order within each transaction,
/// so the report is deterministic for a given pair of inputs.
#[must_use]
pub fn reconcile(local: &[SettlementRecord], facilitator: &[SettlementRecord]) -> LedgerDiff {
    let mut diff = LedgerDiff {
        local_records: local.len(),
        facilitator_records: facilitator.len(),
        ..LedgerDiff::default()
    };

    let local_side = Side::build(local);
    let facilitator_side = Side::build(facilitator);
    diff.skipped_failed = local_side.skipped_failed + facilitator_side.skipped_failed;
    diff.skipped_unidentified =
        local_side.skipped_unidentified + facilitator_side.skipped_unidentified;
    let mut facilitator_by_tx = facilitator_side.by_tx;

    for (key, local_records) in local_side.by_tx {
        // `remove` leaves exactly the keys the local side never mentioned, so
        // the facilitator-only pass below needs no second set lookup.
        let facilitator_records = facilitator_by_tx.remove(&key).unwrap_or_default();

        // Network is part of a settlement's identity, so pair same-network
        // records first instead of trusting row order. A transaction hash can
        // appear on several chains; pairing by position would report two clean
        // settlements as cross-chain mismatches purely because the two exports
        // listed their rows in different orders. For each local record take the
        // *first* unpaired facilitator record on the same network, falling back
        // to any unpaired record only when that network is exhausted. Slots are
        // consumed by index so unpaired facilitator records still come out in
        // input order, preserving the documented output ordering.
        let pairs = local_records.len().min(facilitator_records.len());
        let mut taken = vec![false; facilitator_records.len()];
        let mut choices: Vec<usize> = Vec::with_capacity(pairs);
        for local_record in &local_records[..pairs] {
            let network = local_record.network.trim();
            let same_network = facilitator_records
                .iter()
                .enumerate()
                .find(|(i, rec)| !taken[*i] && rec.network.trim() == network);
            let pick = same_network.or_else(|| {
                facilitator_records
                    .iter()
                    .enumerate()
                    .find(|(i, _)| !taken[*i])
            });
            let Some((index, _)) = pick else {
                unreachable!("pairs <= facilitator records, so an unpaired one exists")
            };
            taken[index] = true;
            choices.push(index);
        }

        for (local_record, fac_index) in local_records[..pairs].iter().zip(choices) {
            let facilitator_record = facilitator_records[fac_index];
            diff.compared += 1;
            // CAIP-2 network is part of a settlement's identity: the same hash
            // on another chain is not the same payment. Check it before the
            // amount — if the chains differ there is no single settlement
            // whose amount could be "correct", so a network finding wins and
            // the pair is not counted as matched either way.
            if local_record.network.trim() != facilitator_record.network.trim() {
                diff.network_mismatch.push(NetworkMismatch {
                    transaction: local_record.transaction.trim().to_owned(),
                    local_network: local_record.network.clone(),
                    facilitator_network: facilitator_record.network.clone(),
                });
            } else if same_amount(&local_record.amount, &facilitator_record.amount) {
                diff.matched += 1;
            } else {
                diff.amount_mismatch.push(AmountMismatch {
                    transaction: local_record.transaction.trim().to_owned(),
                    local_amount: local_record.amount.clone(),
                    facilitator_amount: facilitator_record.amount.clone(),
                });
            }
        }

        // Beyond the shorter side, records are unpaired. This is how a
        // duplicated transaction shows up: it pairs once, and the extra copy
        // becomes a one-sided finding instead of vanishing. Unpaired
        // facilitator records are emitted in their original input order.
        for record in &local_records[pairs..] {
            diff.only_local.push(entry(record));
        }
        for (index, record) in facilitator_records.iter().enumerate() {
            if !taken[index] {
                diff.only_facilitator.push(entry(record));
            }
        }
    }

    for records in facilitator_by_tx.into_values() {
        for record in records {
            diff.only_facilitator.push(entry(record));
        }
    }

    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(transaction: &str, amount: Option<&str>) -> SettlementRecord {
        SettlementRecord {
            success: true,
            error_reason: None,
            error_message: None,
            payer: Some("0xpayer".into()),
            transaction: transaction.into(),
            network: "eip155:2424".into(),
            amount: amount.map(str::to_owned),
            resource: None,
            timestamp: None,
            nonce: None,
        }
    }

    fn failed(transaction: &str) -> SettlementRecord {
        SettlementRecord {
            success: false,
            error_reason: Some("insufficient_funds".into()),
            amount: None,
            ..rec(transaction, None)
        }
    }

    #[test]
    fn identical_ledgers_match_completely() {
        let local = vec![rec("0xaa", Some("1000")), rec("0xbb", Some("2000"))];
        let diff = reconcile(&local, &local);
        assert_eq!(diff.compared, 2);
        assert_eq!(diff.matched, 2);
        assert_eq!(diff.errors(), 0);
        assert!(diff.is_clean());
    }

    #[test]
    fn amount_disagreement_is_reported_with_both_values() {
        let diff = reconcile(&[rec("0xaa", Some("1000"))], &[rec("0xaa", Some("999"))]);
        assert_eq!(diff.matched, 0);
        assert_eq!(
            diff.amount_mismatch,
            vec![AmountMismatch {
                transaction: "0xaa".into(),
                local_amount: Some("1000".into()),
                facilitator_amount: Some("999".into()),
            }]
        );
        assert_eq!(diff.errors(), 1);
    }

    #[test]
    fn one_sided_settlements_are_reported_per_side() {
        let diff = reconcile(&[rec("0xaa", Some("1"))], &[rec("0xbb", Some("2"))]);
        assert_eq!(diff.only_local.len(), 1);
        assert_eq!(diff.only_local[0].transaction, "0xaa");
        assert_eq!(diff.only_facilitator.len(), 1);
        assert_eq!(diff.only_facilitator[0].transaction, "0xbb");
        assert_eq!(diff.errors(), 2);
        assert!(!diff.is_clean());
    }

    #[test]
    fn failed_settlements_are_counted_but_never_reported() {
        // A failure has no on-chain transaction; treating the other side's
        // silence as a discrepancy would double-report every failure.
        let diff = reconcile(&[failed("")], &[failed("")]);
        assert_eq!(diff.skipped_failed, 2);
        assert_eq!(diff.compared, 0);
        assert_eq!(diff.errors(), 0);
        assert!(diff.is_clean());
    }

    #[test]
    fn successful_record_without_a_hash_is_a_warning() {
        let diff = reconcile(&[rec("   ", Some("1"))], &[]);
        assert_eq!(diff.skipped_unidentified, 1);
        assert_eq!(diff.warnings(), 1);
        assert_eq!(diff.errors(), 0);
        assert!(!diff.is_clean(), "an unidentifiable claim must not pass");
    }

    #[test]
    fn hashes_match_regardless_of_case_and_padding() {
        let diff = reconcile(&[rec("  0xAABB  ", Some("1"))], &[rec("0xaabb", Some("1"))]);
        assert_eq!(diff.matched, 1);
        assert_eq!(diff.errors(), 0);
    }

    #[test]
    fn duplicated_transaction_pairs_once_and_surfaces_the_extra() {
        // Local counted the same settlement twice; the facilitator once.
        let diff = reconcile(
            &[rec("0xaa", Some("1000")), rec("0xaa", Some("1000"))],
            &[rec("0xaa", Some("1000"))],
        );
        assert_eq!(diff.compared, 1);
        assert_eq!(diff.matched, 1);
        assert_eq!(diff.only_local.len(), 1);
        assert_eq!(diff.errors(), 1);
    }

    #[test]
    fn equivalent_amount_spellings_are_not_a_mismatch() {
        let diff = reconcile(&[rec("0xaa", Some("1000"))], &[rec("0xaa", Some("01000"))]);
        assert_eq!(diff.matched, 1);
        assert_eq!(diff.errors(), 0);
    }

    #[test]
    fn malformed_amount_never_silently_matches() {
        let diff = reconcile(&[rec("0xaa", Some("1.5"))], &[rec("0xaa", Some("15"))]);
        assert_eq!(diff.matched, 0);
        assert_eq!(diff.amount_mismatch.len(), 1);
    }

    #[test]
    fn absent_amounts_on_both_sides_are_equal() {
        let diff = reconcile(&[rec("0xaa", None)], &[rec("0xaa", None)]);
        assert_eq!(diff.matched, 1);
        assert_eq!(diff.errors(), 0);
    }

    #[test]
    fn empty_ledgers_are_clean() {
        let diff = reconcile(&[], &[]);
        assert_eq!(diff.local_records, 0);
        assert_eq!(diff.facilitator_records, 0);
        assert!(diff.is_clean());
    }

    #[test]
    fn the_same_hash_on_a_different_network_is_not_a_match() {
        // A hash alone does not identify a settlement: CAIP-2 network is part
        // of the identity. Recording a mainnet payment against the testnet (or
        // vice versa) is exactly the kind of mistake reconciliation exists to
        // catch, even when the amount agrees on both sides.
        let local = rec("0xaa", Some("1000"));
        let mut facilitator = rec("0xaa", Some("1000"));
        facilitator.network = "eip155:9999".into();

        let diff = reconcile(&[local], &[facilitator]);
        assert_eq!(diff.compared, 1);
        assert_eq!(diff.matched, 0, "different chains cannot be a clean match");
        assert!(diff.amount_mismatch.is_empty());
        assert_eq!(
            diff.network_mismatch,
            vec![NetworkMismatch {
                transaction: "0xaa".into(),
                local_network: "eip155:2424".into(),
                facilitator_network: "eip155:9999".into(),
            }]
        );
        assert_eq!(diff.errors(), 1);
        assert!(!diff.is_clean(), "a cross-chain mismatch must be a finding");
    }

    #[test]
    fn pairing_prefers_matching_networks_regardless_of_row_order() {
        // Network is part of a settlement's identity (v0.2.1). When one hash
        // has several records on different chains, each side must pair with
        // the record on the SAME chain — not just whichever row came first.
        // Otherwise the same two ledgers yield different verdicts depending
        // only on export order.
        let local_a = rec("0xaa", Some("1000")); // eip155:2424
        let mut local_b = rec("0xaa", Some("2000"));
        local_b.network = "eip155:9999".into();
        let mut fac_b = rec("0xaa", Some("2000"));
        fac_b.network = "eip155:9999".into();
        let fac_a = rec("0xaa", Some("1000")); // eip155:2424

        let in_order = reconcile(
            &[local_a.clone(), local_b.clone()],
            &[fac_a.clone(), fac_b.clone()],
        );
        let shuffled = reconcile(
            &[local_a.clone(), local_b.clone()],
            &[fac_b.clone(), fac_a.clone()],
        );

        assert_eq!(
            in_order.matched, 2,
            "both chains match in the aligned order"
        );
        assert_eq!(in_order.errors(), 0);
        assert_eq!(
            shuffled.matched, 2,
            "same two settlements must still match when facilitator rows are shuffled"
        );
        assert!(
            shuffled.network_mismatch.is_empty(),
            "pairing by row order mislabels two clean matches as cross-chain"
        );
        assert_eq!(shuffled.errors(), 0);
        assert_eq!(
            in_order.matched, shuffled.matched,
            "verdict must not depend on row order"
        );
    }

    #[test]
    fn a_network_mismatch_takes_precedence_over_an_amount_mismatch() {
        // If the chains differ, the two records are not the same settlement,
        // so there is no single "right" amount to compare. Report one precise
        // finding instead of two competing ones.
        let mut local = rec("0xaa", Some("1000"));
        local.network = "eip155:2424".into();
        let mut facilitator = rec("0xaa", Some("999"));
        facilitator.network = "eip155:9999".into();

        let diff = reconcile(&[local], &[facilitator]);
        assert_eq!(diff.network_mismatch.len(), 1);
        assert!(diff.amount_mismatch.is_empty());
        assert_eq!(diff.matched, 0);
        assert_eq!(diff.errors(), 1);
    }

    #[test]
    fn network_whitespace_is_ignored_but_case_is_not() {
        let mut local = rec("0xaa", Some("1"));
        local.network = " eip155:2424 ".into();
        let mut facilitator = rec("0xaa", Some("1"));
        facilitator.network = "eip155:2424".into();
        assert_eq!(reconcile(&[local], &[facilitator.clone()]).matched, 1);

        let mut upper = rec("0xaa", Some("1"));
        upper.network = "EIP155:2424".into();
        // CAIP-2 references can be case-sensitive, so a case difference is
        // surfaced for a human to judge rather than silently folded away.
        assert_eq!(
            reconcile(&[upper], &[facilitator]).network_mismatch.len(),
            1
        );
    }

    #[test]
    fn same_amount_ignores_padding_on_either_side() {
        assert!(
            same_amount(&Some(" 1000 ".into()), &Some("1000".into())),
            "a padded amount is the same atomic value as its trimmed form"
        );
        assert!(same_amount(&Some("01000".into()), &Some("  1000  ".into())));
    }
}
