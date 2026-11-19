//! End-to-end integration tests for the CLI and the public library API.
//!
//! These exercise real binary invocations (via `env!("CARGO_BIN_EXE_...")`)
//! so we cover argument parsing, stdin/stdout and exit codes — not just the
//! library functions in isolation.

use std::io::Write;
use std::process::{Command, Output, Stdio};

use kite_x402_reconciler::ingest::parse_records;
use kite_x402_reconciler::reconcile::{format_atomic, summarize};
use kite_x402_reconciler::sample_records;
use kite_x402_reconciler::validate::validate_all;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kite-x402-reconciler"))
}

/// Run the binary with args and optional stdin, returning its output.
fn run(args: &[&str], stdin: Option<&str>) -> Output {
    let mut command = bin();
    command.args(args);
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn binary");
    if let Some(input) = stdin {
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(input.as_bytes())
            .expect("write stdin");
    }
    child.wait_with_output().expect("wait")
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("utf-8 stdout")
}

#[test]
fn cli_help_lists_all_commands() {
    let output = run(&["--help"], None);
    assert!(output.status.success());
    let text = stdout_text(&output);
    for command in [
        "ingest",
        "validate",
        "summary",
        "duplicates",
        "report",
        "decode",
        "sample",
    ] {
        assert!(text.contains(command), "help missing command: {command}");
    }
}

#[test]
fn cli_sample_emits_parseable_json() {
    let output = run(&["sample", "--format", "json"], None);
    assert!(output.status.success());
    let records = parse_records(&stdout_text(&output)).expect("sample is valid JSON");
    assert_eq!(records.len(), 4);
}

#[test]
fn cli_validate_strict_fails_on_sample() {
    // The sample deliberately contains a duplicate transaction.
    let output = run(&["sample", "--format", "json"], None);
    let ledger = stdout_text(&output);
    let validated = run(&["validate", "--input", "-", "--strict"], Some(&ledger));
    assert!(!validated.status.success(), "--strict should exit non-zero");
    assert!(stdout_text(&validated).contains("duplicate-transaction"));
}

#[test]
fn cli_validate_passes_on_clean_ledger() {
    let clean = r#"[{"success":true,"transaction":"0x1111111111111111111111111111111111111111111111111111111111111111","network":"eip155:2424","amount":"1"}]"#;
    let output = run(&["validate", "--input", "-", "--strict"], Some(clean));
    assert!(
        output.status.success(),
        "clean ledger should pass: {}",
        stdout_text(&output)
    );
}

#[test]
fn cli_summary_via_stdin_shows_totals() {
    let output = run(&["summary", "--input", "-"], Some("[{\"success\":true,\"transaction\":\"0x1\",\"network\":\"eip155:2424\",\"amount\":\"250\"}]"));
    assert!(output.status.success());
    let text = stdout_text(&output);
    assert!(text.contains("total atomic: 250"), "unexpected: {text}");
}

#[test]
fn cli_duplicates_detects_replayed_nonce() {
    let payer = format!("0x{}", "a".repeat(40));
    let json = format!(
        r#"[
          {{"success":true,"transaction":"0x{}","network":"eip155:1","payer":"{payer}","nonce":"n"}},
          {{"success":true,"transaction":"0x{}","network":"eip155:1","payer":"{payer}","nonce":"n"}}
        ]"#,
        "1".repeat(64),
        "2".repeat(64)
    );
    let output = run(
        &["duplicates", "--input", "-", "--format", "json"],
        Some(&json),
    );
    assert!(output.status.success());
    assert!(stdout_text(&output).contains("duplicate-nonce"));
}

#[test]
fn cli_decode_header() {
    use base64::Engine;
    let inner = r#"{"success":true,"transaction":"0x1","network":"eip155:2424","amount":"7"}"#;
    let header = base64::engine::general_purpose::STANDARD.encode(inner);
    let output = run(&["decode", &header], None);
    assert!(output.status.success());
    assert!(stdout_text(&output).contains("\"amount\": \"7\""));
}

#[test]
fn cli_ingest_csv_pipeline() {
    let output = run(&["sample"], None);
    let ledger = stdout_text(&output);
    let csv = run(
        &["ingest", "--input", "-", "--format", "csv"],
        Some(&ledger),
    );
    assert!(csv.status.success());
    let text = stdout_text(&csv);
    assert!(text.starts_with("success,transaction,network"));
    assert_eq!(text.lines().count(), 5); // header + 4 records
}

#[test]
fn cli_report_json_contains_summary_and_issues() {
    let output = run(&["sample"], None);
    let ledger = stdout_text(&output);
    let report = run(
        &["report", "--input", "-", "--format", "json"],
        Some(&ledger),
    );
    assert!(report.status.success());
    let text = stdout_text(&report);
    assert!(text.contains("record_count"));
    assert!(text.contains("duplicate-transaction"));
}

#[test]
fn cli_report_json_is_a_single_parseable_document() {
    // Regression: `report --format json` used to concatenate two independent
    // JSON documents (summary + validation), which no JSON parser accepts.
    let output = run(&["sample"], None);
    let ledger = stdout_text(&output);
    let report = run(
        &["report", "--input", "-", "--format", "json"],
        Some(&ledger),
    );
    assert!(report.status.success());
    let value: serde_json::Value = serde_json::from_str(&stdout_text(&report))
        .expect("report JSON must be one parseable document");
    assert!(value.get("summary").is_some(), "missing summary object");
    assert!(
        value.get("validation").is_some(),
        "missing validation object"
    );
    assert_eq!(value.as_object().map(|m| m.len()), Some(2));
}

#[test]
fn cli_report_csv_has_one_header_and_both_kinds() {
    let output = run(&["sample"], None);
    let ledger = stdout_text(&output);
    let report = run(
        &["report", "--input", "-", "--format", "csv"],
        Some(&ledger),
    );
    assert!(report.status.success());
    let text = stdout_text(&report);
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("section,"))
            .count(),
        1,
        "report CSV must have exactly one header"
    );
    assert!(
        !text.contains("index,code,severity,message"),
        "no second header"
    );
    assert!(text.contains("summary,total"), "summary rows present");
    assert!(text.contains("issue,"), "issue rows present");
    // Every data row must have the same column count as the header.
    let columns = text.lines().next().unwrap().split(',').count();
    for line in text.lines().skip(1).filter(|l| !l.is_empty()) {
        assert_eq!(line.split(',').count(), columns, "ragged CSV row: {line}");
    }
}

// ---- library-level edge cases (boundary tests) ----

#[test]
fn library_sample_has_expected_issues() {
    let report = validate_all(&sample_records());
    assert_eq!(report.errors, 1);
    assert_eq!(report.checked, 4);
}

#[test]
fn library_summary_amounts_are_exact() {
    let summary = summarize(&sample_records());
    assert_eq!(summary.total.amount_atomic, 3_500_000);
    assert_eq!(summary.by_network["eip155:2424"].succeeded, 3);
}

#[test]
fn library_format_atomic_boundaries() {
    assert_eq!(format_atomic(0, 18), "0");
    assert_eq!(format_atomic(1, 18), "0.000000000000000001");
    assert_eq!(format_atomic(u64::MAX as u128, 0), u64::MAX.to_string());
}

// ---- exit-code contract (`--fail-on`) ----
//
// 0 = success, 1 = runtime failure, 2 = usage error (clap), 3 = gate failed.

/// Unique scratch path so concurrently running tests never collide.
fn scratch(name: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("kxr-{}-{name}", std::process::id()));
    path
}

/// A ledger with no errors but one warning (`zero-amount` on a success).
const WARNING_ONLY_LEDGER: &str = r#"[{"success":true,"transaction":"0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","network":"eip155:2424","amount":"0","payer":"0xdddddddddddddddddddddddddddddddddddddddd","timestamp":"2026-11-17T10:00:00Z"}]"#;

#[test]
fn cli_gate_codes_are_distinct_per_failure_kind() {
    let sample = stdout_text(&run(&["sample", "--format", "json"], None));

    // Clean run: no gate requested.
    let ok = run(&["validate", "--input", "-"], Some(&sample));
    assert_eq!(ok.status.code(), Some(0), "default is informational");

    // Gate requested and tripped.
    let gated = run(
        &["validate", "--input", "-", "--fail-on", "error"],
        Some(&sample),
    );
    assert_eq!(
        gated.status.code(),
        Some(3),
        "gate failure has its own code"
    );

    // Runtime failure: unreadable input.
    let missing = scratch("missing.json");
    let runtime = run(&["validate", "--input", missing.to_str().unwrap()], None);
    assert_eq!(
        runtime.status.code(),
        Some(1),
        "io failure is not a gate failure"
    );

    // Usage error: unknown level, rejected by the parser before any work.
    let usage = run(&["validate", "--fail-on", "bogus"], Some(&sample));
    assert_eq!(usage.status.code(), Some(2), "clap keeps exit code 2");
    let stderr = String::from_utf8_lossy(&usage.stderr);
    assert!(stderr.contains("none, error, warning"), "{stderr}");
}

#[test]
fn cli_report_gate_failure_still_writes_the_artifact() {
    let ledger = scratch("gate-ledger.json");
    let report = scratch("gate-report.json");
    let sample = stdout_text(&run(&["sample", "--format", "json"], None));
    std::fs::write(&ledger, sample).expect("write ledger");

    let output = run(
        &[
            "report",
            "--input",
            ledger.to_str().unwrap(),
            "--output",
            report.to_str().unwrap(),
            "--fail-on",
            "error",
        ],
        None,
    );

    assert_eq!(output.status.code(), Some(3));
    // The operator needs the artifact even when the gate trips.
    let body = std::fs::read_to_string(&report).expect("report must be written before gating");
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON report");
    assert!(
        parsed["validation"]["errors"].as_u64().unwrap_or(0) > 0,
        "the sample ledger has a duplicate transaction: {body}"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("gate failed"));
}

#[test]
fn cli_gate_threshold_separates_errors_from_warnings() {
    // One warning, zero errors.
    let lenient = run(
        &["validate", "--input", "-", "--fail-on", "error"],
        Some(WARNING_ONLY_LEDGER),
    );
    assert_eq!(
        lenient.status.code(),
        Some(0),
        "warnings only: error gate passes"
    );

    let strict = run(
        &["validate", "--input", "-", "--fail-on", "warning"],
        Some(WARNING_ONLY_LEDGER),
    );
    assert_eq!(
        strict.status.code(),
        Some(3),
        "warnings only: warning gate trips"
    );
}

#[test]
fn cli_duplicates_honours_the_gate() {
    let sample = stdout_text(&run(&["sample", "--format", "json"], None));
    let plain = run(&["duplicates", "--input", "-"], Some(&sample));
    assert_eq!(plain.status.code(), Some(0));

    let gated = run(
        &["duplicates", "--input", "-", "--fail-on", "error"],
        Some(&sample),
    );
    assert_eq!(gated.status.code(), Some(3));
    assert!(stdout_text(&gated).contains("duplicate-transaction"));
}

#[test]
fn cli_strict_alias_maps_to_fail_on_error() {
    let sample = stdout_text(&run(&["sample", "--format", "json"], None));
    let strict = run(&["validate", "--input", "-", "--strict"], Some(&sample));
    assert_eq!(strict.status.code(), Some(3), "--strict == --fail-on error");

    // `--strict` may only tighten, never relax, an explicit threshold.
    let both = run(
        &["validate", "--input", "-", "--strict", "--fail-on", "none"],
        Some(&sample),
    );
    assert_eq!(both.status.code(), Some(3));
}
