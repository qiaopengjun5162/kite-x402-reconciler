//! Reconciliation: aggregate settlement records by network, payer and day
//! using exact `u128` atomic-unit arithmetic (never floating point).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::{normalize_address, SettlementRecord};

/// Totals for one grouping key.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Totals {
    /// Number of records in the group.
    pub count: usize,
    /// Number of successful settlements.
    pub succeeded: usize,
    /// Number of failed settlements.
    pub failed: usize,
    /// Sum of successfully settled amounts, in atomic units.
    pub amount_atomic: u128,
    /// Number of records whose amount was missing or unparseable.
    pub amount_unavailable: usize,
    /// Number of successful amounts dropped because the running total would
    /// exceed `u128::MAX`. Non-zero means `amount_atomic` could not stay exact.
    pub amount_overflows: usize,
}

impl Totals {
    fn add(&mut self, record: &SettlementRecord) {
        self.count += 1;
        if record.success {
            self.succeeded += 1;
            match record.amount_atomic() {
                Ok(Some(value)) => match self.amount_atomic.checked_add(value) {
                    Some(sum) => self.amount_atomic = sum,
                    None => self.amount_overflows += 1,
                },
                Ok(None) | Err(_) => self.amount_unavailable += 1,
            }
        } else {
            self.failed += 1;
        }
    }
}

/// Aggregated view over a batch of records.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Summary {
    /// Total records processed.
    pub record_count: usize,
    /// Overall totals.
    pub total: Totals,
    /// Breakdown by CAIP-2 network.
    pub by_network: BTreeMap<String, Totals>,
    /// Breakdown by payer address (`(unknown)` when absent).
    pub by_payer: BTreeMap<String, Totals>,
    /// Breakdown by RFC 3339 date (`YYYY-MM-DD`; `(unknown)` when absent).
    pub by_day: BTreeMap<String, Totals>,
}

/// Aggregate a batch of records.
pub fn summarize(records: &[SettlementRecord]) -> Summary {
    let mut summary = Summary {
        record_count: records.len(),
        ..Summary::default()
    };
    for record in records {
        summary.total.add(record);

        // Group by canonical identity, not the raw export spelling. Replay
        // detection, duplicate detection and reconciliation all key payer and
        // network through these same rules; grouping on raw values would split
        // one payer (`0xABCD` / ` 0xabcd `) or one padded CAIP-2 network into
        // separate rows and fragment the totals a machine consumer reads.
        let network = record.network_id().as_str().to_owned();
        summary.by_network.entry(network).or_default().add(record);

        let payer = record
            .payer
            .as_deref()
            .and_then(normalize_address)
            .unwrap_or_else(|| "(unknown)".into());
        summary.by_payer.entry(payer).or_default().add(record);

        // Normalize to UTC before taking the calendar date. Using the
        // record's own offset would split one settlement day across two keys
        // when a ledger mixes timezones (e.g. 23:30-05:00 is already next-day
        // UTC). UTC is the only stable reference across payers.
        let day = record
            .timestamp
            .as_deref()
            .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
            .map(|dt| dt.with_timezone(&Utc).format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "(unknown)".into());
        summary.by_day.entry(day).or_default().add(record);
    }
    summary
}

/// Format an atomic amount using `decimals` places, as an exact decimal string.
///
/// This avoids floating point entirely: value `1234567` with 6 decimals
/// becomes `"1.234567"`.
pub fn format_atomic(value: u128, decimals: u8) -> String {
    let scale = 10u128.checked_pow(decimals as u32);
    let Some(scale) = scale else {
        return value.to_string();
    };
    if scale == 1 {
        return value.to_string();
    }
    let whole = value / scale;
    let fraction = value % scale;
    let fraction_text = format!("{fraction:0width$}", width = decimals as usize);
    let trimmed = fraction_text.trim_end_matches('0');
    if trimmed.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(
        network: &str,
        payer: Option<&str>,
        day: Option<&str>,
        success: bool,
        amount: Option<&str>,
    ) -> SettlementRecord {
        SettlementRecord {
            success,
            error_reason: None,
            error_message: None,
            payer: payer.map(str::to_owned),
            transaction: "0x1".into(),
            network: network.into(),
            amount: amount.map(str::to_owned),
            resource: None,
            timestamp: day.map(|d| format!("{d}T10:00:00Z")),
            nonce: None,
        }
    }

    #[test]
    fn summarizes_totals_and_groups() {
        let records = vec![
            rec(
                "eip155:2424",
                Some("0xpayer"),
                Some("2026-11-17"),
                true,
                Some("1000000"),
            ),
            rec(
                "eip155:2424",
                Some("0xpayer"),
                Some("2026-11-17"),
                true,
                Some("500000"),
            ),
            rec("eip155:1", None, None, false, None),
        ];
        let summary = summarize(&records);
        assert_eq!(summary.record_count, 3);
        assert_eq!(summary.total.count, 3);
        assert_eq!(summary.total.succeeded, 2);
        assert_eq!(summary.total.failed, 1);
        assert_eq!(summary.total.amount_atomic, 1_500_000);
        assert_eq!(summary.by_network["eip155:2424"].amount_atomic, 1_500_000);
        assert_eq!(summary.by_payer["(unknown)"].count, 1);
        assert_eq!(summary.by_day["2026-11-17"].count, 2);
        assert_eq!(summary.by_day["(unknown)"].count, 1);
    }

    #[test]
    fn summary_groups_payer_and_network_by_canonical_identity() {
        // Replay detection and reconciliation both treat these as one payer
        // and one network: payer is case-insensitive hex, network is trimmed.
        // The summary must not split them into duplicate groups, or a machine
        // consumer of by_payer/by_network sees the totals fragmented.
        let records = vec![
            rec(
                "eip155:2424",
                Some("0xABCD"),
                Some("2026-11-17"),
                true,
                Some("100"),
            ),
            rec(
                " eip155:2424 ",
                Some(" 0xabcd "),
                Some("2026-11-17"),
                true,
                Some("200"),
            ),
        ];
        let summary = summarize(&records);
        assert_eq!(
            summary.by_network.len(),
            1,
            "the same CAIP-2 network (with padding) must be one group"
        );
        assert_eq!(summary.by_network["eip155:2424"].amount_atomic, 300);
        assert_eq!(
            summary.by_payer.len(),
            1,
            "the same payer (case/padding variants) must be one group"
        );
        assert_eq!(summary.by_payer["0xabcd"].amount_atomic, 300);
    }

    #[test]
    fn failed_and_invalid_amounts_are_not_summed() {
        let records = vec![
            rec("eip155:1", None, None, false, Some("999")),
            rec("eip155:1", None, None, true, Some("oops")),
            rec("eip155:1", None, None, true, None),
        ];
        let summary = summarize(&records);
        assert_eq!(summary.total.amount_atomic, 0);
        assert_eq!(summary.total.amount_unavailable, 2);
    }

    #[test]
    fn format_atomic_exact() {
        assert_eq!(format_atomic(1_234_567, 6), "1.234567");
        assert_eq!(format_atomic(1_000_000, 6), "1");
        assert_eq!(format_atomic(5, 6), "0.000005");
        assert_eq!(format_atomic(42, 0), "42");
        assert_eq!(format_atomic(123, 3), "0.123");
    }

    #[test]
    fn format_atomic_handles_large_values() {
        assert_eq!(format_atomic(u128::MAX, 0), u128::MAX.to_string());
    }

    #[test]
    fn empty_summary() {
        let summary = summarize(&[]);
        assert_eq!(summary.record_count, 0);
        assert!(summary.by_network.is_empty());
    }

    #[test]
    fn by_day_normalizes_to_utc() {
        // 23:30 at -05:00 is 04:30 next day in UTC; 10:00 at +08:00 is
        // 02:00 the same UTC day. Both must land on the correct UTC date.
        let mut late = rec("eip155:1", None, None, true, Some("100"));
        late.timestamp = Some("2026-11-17T23:30:00-05:00".into());
        let mut early = rec("eip155:1", None, None, true, Some("200"));
        early.timestamp = Some("2026-11-17T10:00:00+08:00".into());

        let summary = summarize(&[late, early]);
        assert_eq!(summary.by_day["2026-11-18"].count, 1);
        assert_eq!(summary.by_day["2026-11-17"].count, 1);
    }

    #[test]
    fn amount_overflow_is_counted_instead_of_panicking() {
        let max = u128::MAX.to_string();
        let records = vec![
            rec("eip155:1", None, None, true, Some(&max)),
            rec("eip155:1", None, None, true, Some("1")),
        ];
        let summary = summarize(&records);
        assert_eq!(summary.total.amount_atomic, u128::MAX);
        assert_eq!(summary.total.amount_overflows, 1);
    }
}
