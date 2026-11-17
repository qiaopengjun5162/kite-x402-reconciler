//! Validation: check settlement records for structural and semantic issues,
//! including duplicate transactions and replayed nonces.

use std::collections::{HashMap, HashSet};

use chrono::DateTime;
use serde::Serialize;

use crate::model::SettlementRecord;

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

/// Look like an EVM address: `0x` followed by 40 hex digits.
fn looks_like_address(value: &str) -> bool {
    let rest = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"));
    match rest {
        Some(hex) => hex.len() == 40 && hex.bytes().all(|b| b.is_ascii_hexdigit()),
        None => false,
    }
}

/// Look like a transaction hash: `0x` followed by 64 hex digits.
fn looks_like_tx_hash(value: &str) -> bool {
    let rest = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"));
    match rest {
        Some(hex) => hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()),
        None => false,
    }
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
        } else if !looks_like_tx_hash(&record.transaction) {
            report.push(Issue {
                index,
                code: "invalid-transaction",
                severity: Severity::Warning,
                message: "transaction hash is not 0x + 64 hex digits".into(),
            });
        }

        // Amount: when present, must parse as u128 and be positive on success.
        if let Some(parsed) = record.amount_atomic() {
            match parsed {
                Ok(value) => {
                    if record.success && value == 0 {
                        report.push(Issue {
                            index,
                            code: "zero-amount",
                            severity: Severity::Warning,
                            message: "successful settlement has zero amount".into(),
                        });
                    }
                }
                Err(_) => report.push(Issue {
                    index,
                    code: "invalid-amount",
                    severity: Severity::Error,
                    message: format!("amount is not a non-negative integer: {:?}", record.amount),
                }),
            }
        }

        // Payer address, when present.
        if let Some(payer) = &record.payer {
            if !looks_like_address(payer) {
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
    let mut first_tx: HashMap<&str, usize> = HashMap::new();
    for (index, record) in records.iter().enumerate() {
        if record.transaction.trim().is_empty() {
            continue;
        }
        if let Some(first) = first_tx.insert(record.transaction.as_str(), index) {
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
    let mut seen_nonces: HashSet<(String, String)> = HashSet::new();
    for (index, record) in records.iter().enumerate() {
        let (Some(payer), Some(nonce)) = (&record.payer, &record.nonce) else {
            continue;
        };
        if !seen_nonces.insert((payer.clone(), nonce.clone())) {
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
