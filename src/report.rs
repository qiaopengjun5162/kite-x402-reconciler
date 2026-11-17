//! Reporting: render validation reports, summaries and records as JSON, CSV
//! or human-readable tables.

use serde::Serialize;

use crate::model::SettlementRecord;
use crate::reconcile::Summary;
use crate::validate::ValidationReport;

/// Output format selected on the CLI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Human-readable table.
    Text,
    /// JSON document.
    Json,
    /// Comma-separated values.
    Csv,
}

impl std::str::FromStr for Format {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "csv" => Ok(Self::Csv),
            other => Err(format!("unknown format: {other} (use text/json/csv)")),
        }
    }
}

/// Escape one field for CSV per RFC 4180.
fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn to_json_pretty<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("serialization to string cannot fail")
}

/// Render a validation report.
pub fn render_validation(report: &ValidationReport, format: Format) -> String {
    match format {
        Format::Json => to_json_pretty(report),
        Format::Csv => {
            let mut out = String::from("index,code,severity,message\n");
            for issue in &report.issues {
                out.push_str(&format!(
                    "{},{},{},{}\n",
                    issue.index,
                    csv_field(issue.code),
                    csv_field(match issue.severity {
                        crate::validate::Severity::Error => "error",
                        crate::validate::Severity::Warning => "warning",
                    }),
                    csv_field(&issue.message)
                ));
            }
            out
        }
        Format::Text => {
            let mut out = format!(
                "Validated {} record(s): {} error(s), {} warning(s)\n",
                report.checked, report.errors, report.warnings
            );
            for issue in &report.issues {
                out.push_str(&format!(
                    "  [{}] #{} {} — {}\n",
                    match issue.severity {
                        crate::validate::Severity::Error => "ERROR",
                        crate::validate::Severity::Warning => "WARN ",
                    },
                    issue.index,
                    issue.code,
                    issue.message
                ));
            }
            out
        }
    }
}

/// Render an aggregated summary.
pub fn render_summary(summary: &Summary, format: Format) -> String {
    match format {
        Format::Json => to_json_pretty(summary),
        Format::Csv => {
            let mut out =
                String::from("group,key,count,succeeded,failed,amount_atomic,amount_unavailable\n");
            for (label, groups) in [
                ("network", &summary.by_network),
                ("payer", &summary.by_payer),
                ("day", &summary.by_day),
            ] {
                for (key, totals) in groups {
                    out.push_str(&format!(
                        "{},{},{},{},{},{},{}\n",
                        label,
                        csv_field(key),
                        totals.count,
                        totals.succeeded,
                        totals.failed,
                        totals.amount_atomic,
                        totals.amount_unavailable
                    ));
                }
            }
            out
        }
        Format::Text => {
            let mut out = format!(
                "Records: {} | succeeded: {} | failed: {} | total atomic: {}\n\n",
                summary.total.count,
                summary.total.succeeded,
                summary.total.failed,
                summary.total.amount_atomic
            );
            out.push_str("By network:\n");
            for (network, totals) in &summary.by_network {
                out.push_str(&format!(
                    "  {:<20} count={:<4} ok={:<4} fail={:<4} atomic={}\n",
                    network, totals.count, totals.succeeded, totals.failed, totals.amount_atomic
                ));
            }
            out.push_str("\nBy day:\n");
            for (day, totals) in &summary.by_day {
                out.push_str(&format!(
                    "  {:<12} count={:<4} atomic={}\n",
                    day, totals.count, totals.amount_atomic
                ));
            }
            out
        }
    }
}

/// Render settlement records (used by `ingest` and `decode`).
pub fn render_records(records: &[SettlementRecord], format: Format) -> String {
    match format {
        Format::Json => {
            if records.len() == 1 {
                to_json_pretty(&records[0])
            } else {
                to_json_pretty(records)
            }
        }
        Format::Csv => {
            let mut out = String::from(
                "success,transaction,network,amount,payer,resource,timestamp,nonce,errorReason,errorMessage\n",
            );
            for record in records {
                let field = |value: &Option<String>| csv_field(value.as_deref().unwrap_or(""));
                out.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{},{}\n",
                    record.success,
                    csv_field(&record.transaction),
                    csv_field(&record.network),
                    field(&record.amount),
                    field(&record.payer),
                    field(&record.resource),
                    field(&record.timestamp),
                    field(&record.nonce),
                    field(&record.error_reason),
                    field(&record.error_message)
                ));
            }
            out
        }
        Format::Text => {
            let mut out = String::new();
            for (index, record) in records.iter().enumerate() {
                out.push_str(&format!(
                    "Record #{index}: {} tx={} network={} amount={:?}\n",
                    if record.success { "OK  " } else { "FAIL" },
                    record.transaction,
                    record.network,
                    record.amount
                ));
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::{Issue, Severity};

    #[test]
    fn csv_field_escapes_special_characters() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_field("a\nb"), "\"a\nb\"");
    }

    #[test]
    fn format_from_str() {
        assert_eq!("json".parse::<Format>().unwrap(), Format::Json);
        assert_eq!("TEXT".parse::<Format>().unwrap(), Format::Text);
        assert_eq!("csv".parse::<Format>().unwrap(), Format::Csv);
        assert!("yaml".parse::<Format>().is_err());
    }

    #[test]
    fn renders_validation_json_and_csv() {
        let report = ValidationReport {
            checked: 1,
            errors: 1,
            warnings: 0,
            issues: vec![Issue {
                index: 0,
                code: "invalid-network",
                severity: Severity::Error,
                message: "bad, network".into(),
            }],
        };
        assert!(render_validation(&report, Format::Json).contains("invalid-network"));
        let csv = render_validation(&report, Format::Csv);
        assert!(csv.starts_with("index,code,severity,message\n"));
        assert!(csv.contains("\"bad, network\""));
    }

    #[test]
    fn renders_summary_text() {
        let mut summary = Summary {
            record_count: 1,
            total: crate::reconcile::Totals {
                count: 1,
                ..crate::reconcile::Totals::default()
            },
            ..Summary::default()
        };
        summary
            .by_network
            .insert("eip155:1".into(), crate::reconcile::Totals::default());
        let text = render_summary(&summary, Format::Text);
        assert!(text.contains("Records: 1"));
        assert!(text.contains("eip155:1"));
    }

    #[test]
    fn renders_records_single_json_is_object() {
        let record = SettlementRecord {
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
        let json = render_records(std::slice::from_ref(&record), Format::Json);
        assert!(json.starts_with('{'));
        let csv = render_records(&[record], Format::Csv);
        assert!(csv.contains("success,transaction,network"));
    }
}
