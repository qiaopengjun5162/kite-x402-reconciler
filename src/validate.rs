//! Validation: check settlement records for structural and semantic issues,
//! including duplicate transactions and replayed nonces.

use std::collections::{HashMap, HashSet};

use chrono::DateTime;
use serde::Serialize;

use crate::model::{normalize_address, normalize_hash, SettlementRecord};

/// How serious a validation issue is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The record is not trustworthy and should not be counted.
    Error,
    /// The record is usable but suspicious.
    Warning,
}

/// A single validation finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Issue {
    /// Zero-based index of the record in the input.
    pub index: usize,
    /// Stable machine-readable code, e.g. `invalid-network`.
    pub code: &'static str,
    /// Severity.
    pub severity: Severity,
    /// Human-readable explanation.
    pub message: String,
}

/// Result of validating a batch of records.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ValidationReport {
    /// Number of records checked.
    pub checked: usize,
    /// Number of error-severity findings.
    pub errors: usize,
    /// Number of warning-severity findings.
    pub warnings: usize,
    /// All findings, in input order.
    pub issues: Vec<Issue>,
}

impl ValidationReport {
    /// Build a report from an already-collected issue set, counting severities
    /// in one place. Callers must not compute `errors`/`warnings` by hand.
    pub fn from_issues(checked: usize, issues: Vec<Issue>) -> Self {
        let errors = issues
            .iter()
            .filter(|issue| issue.severity == Severity::Error)
            .count();
        let warnings = issues
            .iter()
            .filter(|issue| issue.severity == Severity::Warning)
            .count();
        Self {
            checked,
            errors,
            warnings,
            issues,
        }
    }

    fn push(&mut self, issue: Issue) {
        match issue.severity {
            Severity::Error => self.errors += 1,
            Severity::Warning => self.warnings += 1,
        }
        self.issues.push(issue);
    }

    /// True when no error-severity findings were produced.
    pub fn is_ok(&self) -> bool {
        self.errors == 0
    }
}

/// True when `value` is `0x` (or `0X`) followed by exactly `hex_len` hex digits.
fn is_hex_string(value: &str, hex_len: usize) -> bool {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .is_some_and(|hex| hex.len() == hex_len && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Look like an EVM address: `0x` followed by 40 hex digits.
fn looks_like_address(value: &str) -> bool {
    is_hex_string(value, 40)
}

/// Look like a transaction hash: `0x` followed by 64 hex digits.
fn looks_like_tx_hash(value: &str) -> bool {
    is_hex_string(value, 64)
}

/// Validate a batch of records, including cross-record duplicate detection.
pub fn validate_all(records: &[SettlementRecord]) -> ValidationReport {
    let mut report = ValidationReport {
        checked: records.len(),
        ..ValidationReport::default()
    };

    for (index, record) in records.iter().enumerate() {
        // success=false must explain why.
        if !record.success && record.error_reason.is_none() && record.error_message.is_none() {
            report.push(Issue {
                index,
                code: "failure-without-reason",
                severity: Severity::Error,
                message: "failed settlement has no errorReason or errorMessage".into(),
            });
        }

        // success=true should not carry a failure reason (suspicious).
        if record.success && (record.error_reason.is_some() || record.error_message.is_some()) {
            report.push(Issue {
                index,
                code: "success-with-reason",
                severity: Severity::Warning,
                message: "successful settlement carries an error reason".into(),
            });
        }

        // CAIP-2 network.
        if !record.network_id().is_valid() {
            report.push(Issue {
                index,
                code: "invalid-network",
                severity: Severity::Error,
                message: format!("invalid CAIP-2 network: {:?}", record.network),
            });
        }

        // Transaction hash present and well-formed.
        if record.transaction.trim().is_empty() {
            report.push(Issue {
                index,
                code: "missing-transaction",
                severity: Severity::Error,
                message: "transaction hash is empty".into(),
            });
        } else if !looks_like_tx_hash(record.transaction.trim()) {
            report.push(Issue {
                index,
                code: "invalid-transaction",
                severity: Severity::Warning,
                message: "transaction hash is not 0x + 64 hex digits".into(),
            });
        }

        // Amount: when present, must parse as u128 and be positive on success.
        match record.amount_atomic() {
            Ok(Some(value)) => {
                if record.success && value == 0 {
                    report.push(Issue {
                        index,
                        code: "zero-amount",
                        severity: Severity::Warning,
                        message: "successful settlement has zero amount".into(),
                    });
                }
            }
            Ok(None) => {}
            Err(_) => report.push(Issue {
                index,
                code: "invalid-amount",
                severity: Severity::Error,
                message: format!("amount is not a non-negative integer: {:?}", record.amount),
            }),
        }

        // Payer address, when present. Trim for the format check, matching
        // normalize_address used by replay detection and the hash check above.
        if let Some(payer) = &record.payer {
            if !looks_like_address(payer.trim()) {
                report.push(Issue {
                    index,
                    code: "invalid-payer",
                    severity: Severity::Warning,
                    message: "payer is not 0x + 40 hex digits".into(),
                });
            }
        }

        // Timestamp, when present.
        if let Some(ts) = &record.timestamp {
            if DateTime::parse_from_rfc3339(ts).is_err() {
                report.push(Issue {
                    index,
                    code: "invalid-timestamp",
                    severity: Severity::Warning,
                    message: format!("timestamp is not RFC 3339: {ts:?}"),
                });
            }
        }
    }

    append_duplicate_issues(records, &mut report);
    report
}

fn append_duplicate_issues(records: &[SettlementRecord], report: &mut ValidationReport) {
    // Duplicate transaction hashes (error: a tx should settle once).
    // Use the same normalized identity as reconciliation (model::normalize_hash)
    // so that a hash written in a different case or with padding is still
    // recognized as the same settlement — otherwise this check misses exactly
    // the records that reconcile later pairs together.
    let mut first_tx: HashMap<String, usize> = HashMap::new();
    for (index, record) in records.iter().enumerate() {
        let Some(key) = normalize_hash(&record.transaction) else {
            continue;
        };
        if let Some(first) = first_tx.insert(key, index) {
            report.push(Issue {
                index,
                code: "duplicate-transaction",
                severity: Severity::Error,
                message: format!(
                    "transaction repeats the record at index {first}: {:?}",
                    record.transaction
                ),
            });
        }
    }

    // Replayed nonces within the same payer (error: double-spend risk).
    // Normalize the payer address exactly like transaction hashes: EVM
    // addresses are case-insensitive hex, so `0xABCD` and ` 0xabcd ` are one
    // payer and a repeated nonce under either spelling is a single replay.
    let mut seen_nonces: HashSet<(String, String)> = HashSet::new();
    for (index, record) in records.iter().enumerate() {
        let (Some(payer), Some(nonce)) = (&record.payer, &record.nonce) else {
            continue;
        };
        let Some(payer_key) = normalize_address(payer) else {
            continue;
        };
        if !seen_nonces.insert((payer_key, nonce.clone())) {
            report.push(Issue {
                index,
                code: "duplicate-nonce",
                severity: Severity::Error,
                message: format!("payer {payer} reused nonce {nonce:?}"),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(network: &str, tx: &str, amount: Option<&str>) -> SettlementRecord {
        SettlementRecord {
            success: true,
            error_reason: None,
            error_message: None,
            payer: None,
            transaction: tx.into(),
            network: network.into(),
            amount: amount.map(str::to_owned),
            resource: None,
            timestamp: None,
            nonce: None,
        }
    }

    #[test]
    fn accepts_a_clean_record() {
        let report = validate_all(&[record(
            "eip155:2424",
            &format!("0x{}", "a".repeat(64)),
            Some("100"),
        )]);
        assert!(report.is_ok());
        assert_eq!(report.issues.len(), 0);
    }

    #[test]
    fn flags_failure_without_reason() {
        let mut r = record("eip155:1", "0x1", None);
        r.success = false;
        let report = validate_all(&[r]);
        assert!(!report.is_ok());
        assert!(report
            .issues
            .iter()
            .any(|i| i.code == "failure-without-reason"));
    }

    #[test]
    fn flags_invalid_network_and_amount() {
        let report = validate_all(&[record("not-caip2", "0x1", Some("1.5"))]);
        let codes: Vec<_> = report.issues.iter().map(|i| i.code).collect();
        assert!(codes.contains(&"invalid-network"));
        assert!(codes.contains(&"invalid-amount"));
    }

    #[test]
    fn warns_on_success_with_reason_and_zero_amount() {
        let mut r = record("eip155:1", &format!("0x{}", "d".repeat(64)), Some("0"));
        r.error_reason = Some("leftover".into());
        let report = validate_all(&[r]);
        assert!(report.is_ok());
        assert_eq!(report.warnings, 2);
    }

    #[test]
    fn network_validation_trims_padding_like_reconciliation() {
        // `reconcile` trims a CAIP-2 network before comparing two sides (see
        // diff.rs): a hand-edited export with a padded network still pairs as
        // the same chain. Validation must not flag the very records the
        // reconciler accepts, or `validate` and `reconcile` disagree about
        // the same data. Only surrounding padding is tolerated; an embedded
        // space is still rejected by `NetworkId::is_valid`.
        let padded = "  eip155:2424  ";
        let report = validate_all(&[record(padded, &format!("0x{}", "a".repeat(64)), None)]);
        assert!(
            !report.issues.iter().any(|i| i.code == "invalid-network"),
            "a padded network must validate, as reconcile trims it before comparing"
        );
        // Trimming must not widen the shape check: whitespace inside the
        // identifier is still malformed CAIP-2 and stays an error.
        let embedded = "eip 155:1";
        let report = validate_all(&[record(embedded, &format!("0x{}", "b".repeat(64)), None)]);
        assert!(
            report.issues.iter().any(|i| i.code == "invalid-network"),
            "an embedded space must still be rejected"
        );
    }

    #[test]
    fn format_checks_trim_padding_like_the_rest_of_the_pipeline() {
        // reconcile and duplicate detection both trim before comparing hashes
        // and addresses (model::normalize_hash / normalize_address). A clean
        // record padded with whitespace must not be reported as malformed —
        // otherwise validate --fail-on warning rejects a ledger that
        // reconcile considers perfectly clean.
        let mut r = record(
            "eip155:2424",
            &format!("  0x{}  ", "a".repeat(64)),
            Some("100"),
        );
        r.payer = Some(format!("  0x{}  ", "b".repeat(40)));
        let report = validate_all(&[r]);
        assert!(
            !report
                .issues
                .iter()
                .any(|i| i.code == "invalid-transaction"),
            "a padded but valid tx hash must not be flagged"
        );
        assert!(
            !report.issues.iter().any(|i| i.code == "invalid-payer"),
            "a padded but valid payer must not be flagged"
        );
    }

    #[test]
    fn detects_duplicate_transaction() {
        let tx = format!("0x{}", "b".repeat(64));
        let report = validate_all(&[record("eip155:1", &tx, None), record("eip155:1", &tx, None)]);
        assert!(report
            .issues
            .iter()
            .any(|i| i.code == "duplicate-transaction"));
        assert_eq!(report.errors, 1);
    }

    #[test]
    fn duplicate_detection_normalizes_hash_case_and_padding() {
        // `reconcile` normalizes hashes with trim + lowercase before pairing
        // (see diff::match_key), because hex hashes arrive from hand-edited
        // exports and mixed-case logs. Duplicate detection must apply the exact
        // same normalization or it misses the very records reconcile pairs.
        let canonical = format!("0x{}", "b".repeat(64));
        let padded_upper = format!("  {}  ", canonical.to_ascii_uppercase());
        let report = validate_all(&[
            record("eip155:1", &canonical, None),
            record("eip155:1", &padded_upper, None),
        ]);
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.code == "duplicate-transaction"),
            "the same hash in different case/padding must still be a duplicate"
        );
    }

    #[test]
    fn detects_replayed_nonce_per_payer() {
        let payer = format!("0x{}", "c".repeat(40));
        let mut a = record("eip155:1", "0x1", None);
        a.payer = Some(payer.clone());
        a.nonce = Some("n1".into());
        let mut b = record("eip155:1", "0x2", None);
        b.payer = Some(payer);
        b.nonce = Some("n1".into());
        let report = validate_all(&[a, b]);
        assert!(report.issues.iter().any(|i| i.code == "duplicate-nonce"));
    }

    #[test]
    fn duplicate_nonce_detection_normalizes_payer_case_and_padding() {
        // An EVM address is case-insensitive hex: `0xAB..` and ` 0xab.. `
        // denote the same payer, and `looks_like_address` accepts both.
        // Replay detection must normalize the payer exactly like the
        // transaction hash, or a reused nonce slips through when the address
        // is written in a different case or with surrounding whitespace.
        let canonical = format!("0x{}", "c".repeat(40));
        let padded_upper = format!("  {}  ", canonical.to_ascii_uppercase());
        let mut a = record("eip155:1", "0x1", None);
        a.payer = Some(canonical);
        a.nonce = Some("n1".into());
        let mut b = record("eip155:1", "0x2", None);
        b.payer = Some(padded_upper);
        b.nonce = Some("n1".into());
        let report = validate_all(&[a, b]);
        assert!(
            report.issues.iter().any(|i| i.code == "duplicate-nonce"),
            "the same payer in different case/padding must still be a reused nonce"
        );
    }

    #[test]
    fn same_nonce_different_payer_is_fine() {
        let mut a = record("eip155:1", "0x1", None);
        a.payer = Some(format!("0x{}", "1".repeat(40)));
        a.nonce = Some("n1".into());
        let mut b = record("eip155:1", "0x2", None);
        b.payer = Some(format!("0x{}", "2".repeat(40)));
        b.nonce = Some("n1".into());
        assert!(validate_all(&[a, b]).is_ok());
    }

    #[test]
    fn empty_batch_is_ok() {
        let report = validate_all(&[]);
        assert!(report.is_ok());
        assert_eq!(report.checked, 0);
    }
}
