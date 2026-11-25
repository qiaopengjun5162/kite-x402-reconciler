//! Reporting: render validation reports, summaries and records as JSON, CSV,
//! HTML or human-readable tables.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::diff::LedgerDiff;
use crate::model::SettlementRecord;
use crate::reconcile::{Summary, Totals};
use crate::validate::{Severity, ValidationReport};

/// Output format selected on the CLI.
///
/// Adding a variant is a semver-minor change for callers that exhaustively
/// match on this enum; the CLI itself handles every variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Human-readable table.
    Text,
    /// JSON document.
    Json,
    /// Comma-separated values.
    Csv,
    /// Self-contained HTML document, viewable by double-clicking the file.
    Html,
}

impl std::str::FromStr for Format {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "csv" => Ok(Self::Csv),
            "html" => Ok(Self::Html),
            other => Err(format!("unknown format: {other} (use text/json/csv/html)")),
        }
    }
}

/// Render an optional field for human output without leaking Rust's `Option`
/// debug formatting (`Some("999")`) into a CLI the operator reads.
fn shown(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("(none)")
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

/// Escape a value before embedding it in HTML.
///
/// Settlement records are external input (hand-edited exports, header
/// decodes). Rendering a `payer` or `errorMessage` containing `<script>` into
/// a report would turn the report into an XSS vector when opened in a
/// browser, so every dynamic value must pass through here first.
fn html_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Inline stylesheet so the report is a single self-contained file: no outer
/// CSS, no JS, no network. Colors are chosen to be legible on a light page.
const HTML_STYLE: &str = ":root{color-scheme:light}\
body{margin:0;padding:24px;font:14px/1.6 -apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif;color:#1f2937;background:#f9fafb}\
main{max-width:960px;margin:0 auto}h1{font-size:22px;margin:0 0 4px}\
h2{font-size:16px;margin:28px 0 10px}p.sub{color:#6b7280;margin:0 0 20px}\
.cards{display:flex;gap:12px;flex-wrap:wrap;margin:16px 0 8px}\
.card{flex:1 1 140px;background:#fff;border:1px solid #e5e7eb;border-radius:10px;padding:12px 16px}\
.card .k{font-size:12px;color:#6b7280}.card .v{font-size:22px;font-weight:600;margin-top:4px}\
.card.err .v{color:#b91c1c}.card.warn .v{color:#b45309}.card.ok .v{color:#047857}\
table{border-collapse:collapse;width:100%;background:#fff;border:1px solid #e5e7eb;border-radius:8px;overflow:hidden}\
th,td{text-align:left;padding:8px 12px;border-bottom:1px solid #f3f4f6}\
th{background:#f9fafb;color:#4b5563;font-weight:600;font-size:12px}\n\
tr:last-child td{border-bottom:0}\n\
.tag{display:inline-block;padding:1px 8px;border-radius:999px;font-size:12px;font-weight:600}\n\
.tag.error{background:#fee2e2;color:#991b1b}.tag.warning{background:#fef3c7;color:#92400e}\n\
.note{background:#fef3c7;border:1px solid #fcd34d;border-radius:8px;padding:10px 14px;margin:12px 0}\n\
code{font:12px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace}\n\
.empty{color:#6b7280;font-style:italic}\n";

/// Wrap a body fragment in a complete HTML5 document.
fn html_document(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<style>\n{}\n</style>\n</head>\n<body>\n<main>\n{}\n</main>\n</body>\n</html>\n",
        html_escape(title),
        HTML_STYLE,
        body
    )
}

/// One summary card (label + value, with an optional color class).
fn html_card(label: &str, value: &str, class: &str) -> String {
    format!(
        "<div class=\"card {}\"><div class=\"k\">{}</div><div class=\"v\">{}</div></div>",
        class,
        html_escape(label),
        html_escape(value)
    )
}

/// Render the overview cards shared by `report` and `summary`.
fn html_summary_cards(summary: &Summary) -> String {
    let mut out = String::from("<div class=\"cards\">");
    out.push_str(&html_card("Records", &summary.total.count.to_string(), ""));
    out.push_str(&html_card(
        "Succeeded",
        &summary.total.succeeded.to_string(),
        "ok",
    ));
    out.push_str(&html_card(
        "Failed",
        &summary.total.failed.to_string(),
        if summary.total.failed > 0 { "warn" } else { "" },
    ));
    out.push_str(&html_card(
        "Total (atomic)",
        &summary.total.amount_atomic.to_string(),
        "",
    ));
    out.push_str("</div>");
    if summary.total.amount_overflows > 0 {
        out.push_str(&format!(
            "<div class=\"note\">WARNING: {} amount(s) exceeded u128::MAX and were NOT counted; the total is incomplete.</div>",
            summary.total.amount_overflows
        ));
    }
    out
}

/// Render one network/payer/day group as an HTML table.
fn html_totals_table(groups: &BTreeMap<String, Totals>) -> String {
    if groups.is_empty() {
        return "<p class=\"empty\">No records.</p>".to_owned();
    }
    let mut out = String::from(
        "<table><thead><tr><th>Key</th><th>Count</th><th>OK</th><th>Fail</th><th>Amount (atomic)</th><th>No amount</th><th>Overflow</th></tr></thead><tbody>",
    );
    for (key, totals) in groups {
        out.push_str(&format!(
            "<tr><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            html_escape(key),
            totals.count,
            totals.succeeded,
            totals.failed,
            totals.amount_atomic,
            totals.amount_unavailable,
            totals.amount_overflows
        ));
    }
    out.push_str("</tbody></table>");
    out
}

/// Render already-escaped rows under plain headers.
///
/// Callers escape each cell with [`html_escape`] before passing them here;
/// the header strings are compile-time literals owned by this crate.
fn html_rows_table<const N: usize>(headers: &[&str], rows: &[[String; N]]) -> String {
    if rows.is_empty() {
        return "<p class=\"empty\">None.</p>".to_owned();
    }
    let mut out = String::from("<table><thead><tr>");
    for header in headers {
        out.push_str(&format!("<th>{}</th>", html_escape(header)));
    }
    out.push_str("</tr></thead><tbody>");
    for row in rows {
        out.push_str("<tr>");
        for cell in row {
            out.push_str(&format!("<td>{}</td>", cell));
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table>");
    out
}

/// Render validation findings as an HTML table with colored severity tags.
fn html_issues_table(report: &ValidationReport) -> String {
    if report.issues.is_empty() {
        return "<p class=\"empty\">No findings — this ledger is clean.</p>".to_owned();
    }
    let mut out = String::from(
        "<table><thead><tr><th>#</th><th>Severity</th><th>Code</th><th>Message</th></tr></thead><tbody>",
    );
    for issue in &report.issues {
        let (tag, class) = match issue.severity {
            Severity::Error => ("error", "error"),
            Severity::Warning => ("warning", "warning"),
        };
        out.push_str(&format!(
            "<tr><td>{}</td><td><span class=\"tag {}\">{}</span></td><td><code>{}</code></td><td>{}</td></tr>",
            issue.index,
            class,
            tag,
            html_escape(issue.code),
            html_escape(&issue.message)
        ));
    }
    out.push_str("</tbody></table>");
    out
}

/// Render a validation report.
pub fn render_validation(report: &ValidationReport, format: Format) -> String {
    match format {
        Format::Json => to_json_pretty(report),
        Format::Html => {
            let mut body = String::from("<h1>Validation report</h1>");
            body.push_str(&format!(
                "<p class=\"sub\">Checked {} record(s)</p><div class=\"cards\">",
                report.checked
            ));
            body.push_str(&html_card(
                "Errors",
                &report.errors.to_string(),
                if report.errors > 0 { "err" } else { "" },
            ));
            body.push_str(&html_card(
                "Warnings",
                &report.warnings.to_string(),
                if report.warnings > 0 { "warn" } else { "" },
            ));
            body.push_str("</div>");
            body.push_str(&html_issues_table(report));
            html_document("Validation report", &body)
        }
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
        Format::Html => {
            let mut body = String::from("<h1>Settlement summary</h1>");
            body.push_str("<p class=\"sub\">Exact u128 atomic-unit totals · days in UTC</p>");
            body.push_str(&html_summary_cards(summary));
            body.push_str("<h2>By network</h2>");
            body.push_str(&html_totals_table(&summary.by_network));
            body.push_str("<h2>By payer</h2>");
            body.push_str(&html_totals_table(&summary.by_payer));
            body.push_str("<h2>By day (UTC)</h2>");
            body.push_str(&html_totals_table(&summary.by_day));
            html_document("Settlement summary", &body)
        }
        Format::Csv => {
            let mut out = String::from(
                "group,key,count,succeeded,failed,amount_atomic,amount_unavailable,amount_overflows\n",
            );
            for (label, groups) in [
                ("network", &summary.by_network),
                ("payer", &summary.by_payer),
                ("day", &summary.by_day),
            ] {
                for (key, totals) in groups {
                    out.push_str(&format!(
                        "{},{},{},{},{},{},{},{}\n",
                        label,
                        csv_field(key),
                        totals.count,
                        totals.succeeded,
                        totals.failed,
                        totals.amount_atomic,
                        totals.amount_unavailable,
                        totals.amount_overflows
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
            if summary.total.amount_overflows > 0 {
                out.push_str(&format!(
                    "WARNING: {} amount(s) exceeded u128::MAX and were NOT counted; total is incomplete.\n\n",
                    summary.total.amount_overflows
                ));
            }
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

/// Render a complete reconciliation report (summary + validation issues) as a
/// single document.
///
/// For JSON this is one object with `summary` and `validation` keys — NOT two
/// concatenated JSON documents (which would be invalid JSON and unparseable).
/// Text and CSV keep the historic two-section layout.
pub fn render_full_report(
    summary: &Summary,
    validation: &ValidationReport,
    format: Format,
) -> String {
    #[derive(Serialize)]
    struct FullReport<'a> {
        summary: &'a Summary,
        validation: &'a ValidationReport,
    }

    match format {
        Format::Json => to_json_pretty(&FullReport {
            summary,
            validation,
        }),
        Format::Html => {
            let mut body = String::from("<h1>x402 settlement report</h1>");
            body.push_str("<p class=\"sub\">Offline audit · exact atomic-unit money · no external resources</p>");
            body.push_str(&html_summary_cards(summary));
            body.push_str("<div class=\"cards\">");
            body.push_str(&html_card(
                "Errors",
                &validation.errors.to_string(),
                if validation.errors > 0 { "err" } else { "" },
            ));
            body.push_str(&html_card(
                "Warnings",
                &validation.warnings.to_string(),
                if validation.warnings > 0 { "warn" } else { "" },
            ));
            body.push_str("</div>");
            body.push_str("<h2>By network</h2>");
            body.push_str(&html_totals_table(&summary.by_network));
            body.push_str("<h2>By payer</h2>");
            body.push_str(&html_totals_table(&summary.by_payer));
            body.push_str("<h2>By day (UTC)</h2>");
            body.push_str(&html_totals_table(&summary.by_day));
            body.push_str("<h2>Validation findings</h2>");
            body.push_str(&html_issues_table(validation));
            html_document("x402 settlement report", &body)
        }
        Format::Csv => render_full_report_csv(summary, validation),
        // Text is for humans, so two labelled sections read better than one
        // wide table. CSV/JSON are machine-facing and stay single-structure.
        Format::Text => {
            let mut body = render_summary(summary, format);
            body.push('\n');
            body.push_str(&render_validation(validation, format));
            body
        }
    }
}

/// One flat CSV table for the whole report: a `section` column marks each row
/// as a summary group or an issue, so a single header covers every row.
/// Concatenating two CSV tables with different headers was not parseable.
fn render_full_report_csv(summary: &Summary, validation: &ValidationReport) -> String {
    fn csv_row(fields: &[String]) -> String {
        let mut line = fields.join(",");
        line.push('\n');
        line
    }

    let mut out = String::from(
        "section,group,key,count,succeeded,failed,amount_atomic,amount_unavailable,amount_overflows,issue_index,issue_code,issue_severity,issue_message\n",
    );

    let mut summary_row = |label: &str, key: &str, totals: &Totals| {
        out.push_str(&csv_row(&[
            "summary".into(),
            label.into(),
            csv_field(key),
            totals.count.to_string(),
            totals.succeeded.to_string(),
            totals.failed.to_string(),
            totals.amount_atomic.to_string(),
            totals.amount_unavailable.to_string(),
            totals.amount_overflows.to_string(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ]));
    };

    summary_row("total", "(all)", &summary.total);
    for (label, groups) in [
        ("network", &summary.by_network),
        ("payer", &summary.by_payer),
        ("day", &summary.by_day),
    ] {
        for (key, totals) in groups {
            summary_row(label, key, totals);
        }
    }

    for issue in &validation.issues {
        out.push_str(&csv_row(&[
            "issue".into(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            issue.index.to_string(),
            csv_field(issue.code),
            csv_field(match issue.severity {
                crate::validate::Severity::Error => "error",
                crate::validate::Severity::Warning => "warning",
            }),
            csv_field(&issue.message),
        ]));
    }
    out
}

/// Render a local-vs-facilitator reconciliation.
///
/// JSON is the whole [`LedgerDiff`]; CSV is a flat findings table with a
/// `section` column, so one header covers all four finding kinds. Totals live
/// in JSON and in the text header — CSV carries only rows that need action.
pub fn render_diff(diff: &LedgerDiff, format: Format) -> String {
    match format {
        Format::Json => to_json_pretty(diff),
        Format::Html => {
            let mut body = String::from("<h1>Ledger reconciliation</h1>");
            body.push_str("<p class=\"sub\">Local records vs facilitator records</p>");
            body.push_str("<div class=\"cards\">");
            body.push_str(&html_card("Matched", &diff.matched.to_string(), "ok"));
            body.push_str(&html_card(
                "Discrepancies",
                &diff.errors().to_string(),
                if diff.errors() > 0 { "err" } else { "" },
            ));
            body.push_str(&html_card(
                "Only local",
                &diff.only_local.len().to_string(),
                "warn",
            ));
            body.push_str(&html_card(
                "Only facilitator",
                &diff.only_facilitator.len().to_string(),
                "warn",
            ));
            body.push_str("</div>");
            if diff.skipped_unidentified > 0 {
                body.push_str(&format!(
                    "<div class=\"note\">WARNING: {} successful record(s) had no transaction hash and were NOT reconciled.</div>",
                    diff.skipped_unidentified
                ));
            }
            body.push_str(&format!(
                "<p class=\"sub\">Compared {} pair(s) · skipped {} failed settlement(s)</p>",
                diff.compared, diff.skipped_failed
            ));
            let amount_rows: Vec<[String; 3]> = diff
                .amount_mismatch
                .iter()
                .map(|m| {
                    [
                        html_escape(&m.transaction),
                        html_escape(shown(&m.local_amount)),
                        html_escape(shown(&m.facilitator_amount)),
                    ]
                })
                .collect();
            let network_rows: Vec<[String; 3]> = diff
                .network_mismatch
                .iter()
                .map(|m| {
                    [
                        html_escape(&m.transaction),
                        html_escape(&m.local_network),
                        html_escape(&m.facilitator_network),
                    ]
                })
                .collect();
            body.push_str("<h2>Amount mismatches</h2>");
            body.push_str(&html_rows_table(
                &["Transaction", "Local amount", "Facilitator amount"],
                &amount_rows,
            ));
            body.push_str("<h2>Network mismatches</h2>");
            body.push_str(&html_rows_table(
                &["Transaction", "Local network", "Facilitator network"],
                &network_rows,
            ));
            for (label, entries) in [
                ("Only in local", &diff.only_local),
                ("Only in facilitator", &diff.only_facilitator),
            ] {
                let rows: Vec<[String; 4]> = entries
                    .iter()
                    .map(|e| {
                        [
                            html_escape(&e.transaction),
                            html_escape(&e.network),
                            html_escape(shown(&e.amount)),
                            html_escape(shown(&e.payer)),
                        ]
                    })
                    .collect();
                body.push_str(&format!("<h2>{label}</h2>"));
                body.push_str(&html_rows_table(
                    &["Transaction", "Network", "Amount", "Payer"],
                    &rows,
                ));
            }
            html_document("Ledger reconciliation", &body)
        }
        Format::Csv => {
            let mut out = String::from(
                "section,transaction,local_amount,facilitator_amount,network,facilitator_network,payer\n",
            );
            let optional = |value: &Option<String>| csv_field(value.as_deref().unwrap_or(""));
            for mismatch in &diff.amount_mismatch {
                out.push_str(&format!(
                    "amount_mismatch,{},{},{},,,\n",
                    csv_field(&mismatch.transaction),
                    optional(&mismatch.local_amount),
                    optional(&mismatch.facilitator_amount)
                ));
            }
            for mismatch in &diff.network_mismatch {
                out.push_str(&format!(
                    "network_mismatch,{},,,{},{},\n",
                    csv_field(&mismatch.transaction),
                    csv_field(&mismatch.local_network),
                    csv_field(&mismatch.facilitator_network)
                ));
            }
            for (section, entries) in [
                ("only_local", &diff.only_local),
                ("only_facilitator", &diff.only_facilitator),
            ] {
                for entry in entries {
                    let amount = optional(&entry.amount);
                    let (local, facilitator) = if section == "only_local" {
                        (amount.clone(), String::new())
                    } else {
                        (String::new(), amount)
                    };
                    out.push_str(&format!(
                        "{section},{},{local},{facilitator},{},,{payer}\n",
                        csv_field(&entry.transaction),
                        csv_field(&entry.network),
                        payer = optional(&entry.payer)
                    ));
                }
            }
            out
        }
        Format::Text => {
            let mut out = format!(
                "Local records: {} | facilitator records: {} | compared: {}\n\
                 Matched: {} | amount mismatches: {} | network mismatches: {} | only local: {} | only facilitator: {}\n\
                 Skipped: {} failed (no on-chain settlement), {} without a transaction hash\n",
                diff.local_records,
                diff.facilitator_records,
                diff.compared,
                diff.matched,
                diff.amount_mismatch.len(),
                diff.network_mismatch.len(),
                diff.only_local.len(),
                diff.only_facilitator.len(),
                diff.skipped_failed,
                diff.skipped_unidentified
            );
            if diff.skipped_unidentified > 0 {
                out.push_str(
                    "WARNING: successful records without a transaction hash were NOT reconciled.\n",
                );
            }

            out.push_str("\nAmount mismatches:\n");
            for mismatch in &diff.amount_mismatch {
                out.push_str(&format!(
                    "  {} local={} facilitator={}\n",
                    mismatch.transaction,
                    shown(&mismatch.local_amount),
                    shown(&mismatch.facilitator_amount)
                ));
            }
            out.push_str("\nNetwork mismatches:\n");
            for mismatch in &diff.network_mismatch {
                out.push_str(&format!(
                    "  {} local={} facilitator={}\n",
                    mismatch.transaction, mismatch.local_network, mismatch.facilitator_network
                ));
            }
            for (label, entries) in [
                ("Only in local:", &diff.only_local),
                ("Only in facilitator:", &diff.only_facilitator),
            ] {
                out.push_str(&format!("\n{label}\n"));
                for entry in entries {
                    out.push_str(&format!(
                        "  {} network={} amount={} payer={}\n",
                        entry.transaction,
                        entry.network,
                        shown(&entry.amount),
                        shown(&entry.payer)
                    ));
                }
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
        Format::Html => {
            let mut body = String::from("<h1>Settlement records</h1>");
            body.push_str(&format!("<p class=\"sub\">{} record(s)</p>", records.len()));
            let rows: Vec<[String; 6]> = records
                .iter()
                .map(|r| {
                    [
                        if r.success { "OK" } else { "FAIL" }.to_owned(),
                        html_escape(&r.transaction),
                        html_escape(&r.network),
                        html_escape(shown(&r.amount)),
                        html_escape(shown(&r.payer)),
                        html_escape(shown(&r.error_reason)),
                    ]
                })
                .collect();
            body.push_str(&html_rows_table(
                &[
                    "Status",
                    "Transaction",
                    "Network",
                    "Amount",
                    "Payer",
                    "Error reason",
                ],
                &rows,
            ));
            html_document("Settlement records", &body)
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
                    "Record #{index}: {} tx={} network={} amount={}\n",
                    if record.success { "OK  " } else { "FAIL" },
                    record.transaction,
                    record.network,
                    shown(&record.amount)
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
    fn csv_renders_amount_overflows_like_text_and_json() {
        // text prints a WARNING and JSON serializes amount_overflows, so a
        // machine CSV consumer must see it too — otherwise a dropped,
        // unrepresentable amount leaves a plausible-but-wrong total with no
        // indication that it is incomplete.
        let totals = Totals {
            count: 2,
            succeeded: 2,
            amount_atomic: u128::MAX,
            amount_overflows: 1,
            ..Totals::default()
        };
        let summary = Summary {
            record_count: 2,
            total: totals.clone(),
            by_network: std::collections::BTreeMap::from([("eip155:1".into(), totals)]),
            ..Summary::default()
        };

        let summary_csv = render_summary(&summary, Format::Csv);
        assert!(
            summary_csv
                .lines()
                .next()
                .unwrap()
                .contains("amount_overflows"),
            "summary CSV header must expose amount_overflows"
        );
        assert!(summary_csv.contains(",1\n") || summary_csv.contains(",1\r\n"));

        let full_csv = render_full_report_csv(&summary, &ValidationReport::default());
        assert!(
            full_csv
                .lines()
                .next()
                .unwrap()
                .contains("amount_overflows"),
            "full report CSV header must expose amount_overflows"
        );
    }

    #[test]
    fn format_from_str() {
        assert_eq!("json".parse::<Format>().unwrap(), Format::Json);
        assert_eq!("TEXT".parse::<Format>().unwrap(), Format::Text);
        assert_eq!("csv".parse::<Format>().unwrap(), Format::Csv);
        assert_eq!("HTML".parse::<Format>().unwrap(), Format::Html);
        assert!("yaml".parse::<Format>().is_err());
    }

    #[test]
    fn html_escape_neutralizes_every_dangerous_character() {
        assert_eq!(
            html_escape("<script>alert('x')</script> & \"q\""),
            "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; &quot;q&quot;"
        );
        assert_eq!(html_escape("plain"), "plain");
    }

    #[test]
    fn html_full_report_escapes_hostile_record_fields() {
        // A ledger field is external input. If a `<script>` in an error
        // message reached the HTML raw, opening the report would execute it.
        let validation = ValidationReport {
            checked: 1,
            errors: 1,
            warnings: 0,
            issues: vec![Issue {
                index: 0,
                code: "invalid-network",
                severity: Severity::Error,
                message: "<script>alert(1)</script> & 'x'".into(),
            }],
        };
        let mut by_payer = BTreeMap::new();
        by_payer.insert("<img src=x onerror=alert(1)>".into(), Totals::default());
        let summary = Summary {
            record_count: 1,
            by_payer,
            ..Summary::default()
        };
        let html = render_full_report(&summary, &validation, Format::Html);
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(
            !html.contains("<script>"),
            "raw script tag leaked into report"
        );
        assert!(
            !html.contains("<img src=x"),
            "raw img tag leaked into report"
        );
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
        assert!(html.contains("&amp; &#39;x&#39;"));
    }

    #[test]
    fn html_renderers_emit_standalone_documents() {
        let summary = Summary {
            record_count: 1,
            ..Summary::default()
        };
        let validation = ValidationReport::default();
        let diff = LedgerDiff::default();
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
        for html in [
            render_validation(&validation, Format::Html),
            render_summary(&summary, Format::Html),
            render_full_report(&summary, &validation, Format::Html),
            render_diff(&diff, Format::Html),
            render_records(std::slice::from_ref(&record), Format::Html),
        ] {
            assert!(html.starts_with("<!DOCTYPE html>"));
            assert!(html.contains("<style>"), "CSS must be inlined");
            assert!(html.trim_end().ends_with("</html>"));
            assert!(!html.contains("http://"), "no external resources");
            assert!(!html.contains("https://"), "no external resources");
        }
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
