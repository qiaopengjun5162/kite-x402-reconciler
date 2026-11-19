//! CLI for `kite-x402-reconciler`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kite_x402_reconciler::gate::FailOn;
use kite_x402_reconciler::ingest::{decode_payment_response, read_records};
use kite_x402_reconciler::reconcile::summarize;
use kite_x402_reconciler::report::{
    render_full_report, render_records, render_summary, render_validation, Format,
};
use kite_x402_reconciler::sample_records;
use kite_x402_reconciler::validate::{validate_all, ValidationReport};

/// Exit status when the run finished and the ledger failed the requested gate.
///
/// The full contract is `0` success, `1` runtime failure (unreadable input,
/// malformed ledger), `2` usage error (clap's own code) and `3` gate failed.
/// Keeping the gate off `2` matters: a wrapper that treated a typo in the
/// command line the same as a dirty ledger would alert on the wrong thing.
const EXIT_GATE_FAILED: u8 = 3;

#[derive(Debug, Parser)]
#[command(
    name = "kite-x402-reconciler",
    version,
    about = "Reconcile and audit Kite x402 settlement records"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Read and normalize settlement records from a file or stdin.
    Ingest {
        /// Input file (`-` or omitted for stdin). JSON array, JSONL, or one object.
        #[arg(short, long)]
        input: Option<PathBuf>,
        /// Output format.
        #[arg(short, long, default_value = "json")]
        format: Format,
    },
    /// Validate records and report issues (including duplicates/replays).
    Validate {
        #[arg(short, long)]
        input: Option<PathBuf>,
        #[arg(short, long, default_value = "text")]
        format: Format,
        /// Deprecated alias for `--fail-on error`.
        #[arg(long, hide = true)]
        strict: bool,
        /// Exit with status 3 when findings reach this severity
        /// (`none`, `error`, `warning`).
        #[arg(long, default_value = "none", value_parser = FailOn::from_str)]
        fail_on: FailOn,
    },
    /// Aggregate records by network, payer and day.
    Summary {
        #[arg(short, long)]
        input: Option<PathBuf>,
        #[arg(short, long, default_value = "text")]
        format: Format,
    },
    /// Detect duplicate transactions and replayed nonces in one step.
    Duplicates {
        #[arg(short, long)]
        input: Option<PathBuf>,
        #[arg(short, long, default_value = "text")]
        format: Format,
        /// Exit with status 3 when findings reach this severity
        /// (`none`, `error`, `warning`).
        #[arg(long, default_value = "none", value_parser = FailOn::from_str)]
        fail_on: FailOn,
    },
    /// Write a full reconciliation report (summary + issues) to a file.
    Report {
        #[arg(short, long)]
        input: Option<PathBuf>,
        /// Output path; `-` for stdout.
        #[arg(short, long, default_value = "-")]
        output: PathBuf,
        #[arg(short, long, default_value = "json")]
        format: Format,
        /// Exit with status 3 when findings reach this severity
        /// (`none`, `error`, `warning`). The report is still written before the
        /// status is reported.
        #[arg(long, default_value = "none", value_parser = FailOn::from_str)]
        fail_on: FailOn,
    },
    /// Decode one PAYMENT-RESPONSE header (base64 JSON) into a record.
    Decode {
        /// The base64 header value. If omitted, read it from stdin.
        header: Option<String>,
        #[arg(short, long, default_value = "json")]
        format: Format,
    },
    /// Print a sample ledger (JSON array) for demos and piping.
    Sample {
        #[arg(short, long, default_value = "json")]
        format: Format,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            // `{:#}` keeps the anyhow context chain readable on one line.
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    match cli.command {
        Command::Ingest { input, format } => {
            let records = read_records(input.as_deref()).context("failed to read records")?;
            print!("{}", render_records(&records, format));
            Ok(ExitCode::SUCCESS)
        }
        Command::Validate {
            input,
            format,
            strict,
            fail_on,
        } => {
            let records = read_records(input.as_deref()).context("failed to read records")?;
            let report = validate_all(&records);
            print!("{}", render_validation(&report, format));
            // `--strict` can only tighten the threshold, never relax `--fail-on`.
            let policy = if strict {
                fail_on.max(FailOn::Error)
            } else {
                fail_on
            };
            Ok(gate(policy, &report))
        }
        Command::Summary { input, format } => {
            let records = read_records(input.as_deref()).context("failed to read records")?;
            let summary = summarize(&records);
            print!("{}", render_summary(&summary, format));
            Ok(ExitCode::SUCCESS)
        }
        Command::Duplicates {
            input,
            format,
            fail_on,
        } => {
            let records = read_records(input.as_deref()).context("failed to read records")?;
            let report = validate_all(&records);
            let duplicates: Vec<_> = report
                .issues
                .into_iter()
                .filter(|issue| matches!(issue.code, "duplicate-transaction" | "duplicate-nonce"))
                .collect();
            let filtered = ValidationReport::from_issues(records.len(), duplicates);
            print!("{}", render_validation(&filtered, format));
            Ok(gate(fail_on, &filtered))
        }
        Command::Report {
            input,
            output,
            format,
            fail_on,
        } => {
            let records = read_records(input.as_deref()).context("failed to read records")?;
            let summary = summarize(&records);
            let report = validate_all(&records);
            let body = render_full_report(&summary, &report, format);
            if output.as_os_str() == "-" {
                print!("{body}");
            } else {
                std::fs::write(&output, body)
                    .with_context(|| format!("failed to write {}", output.display()))?;
                eprintln!("report written to {}", output.display());
            }
            // Gate after the artifact exists: a dirty ledger must still leave
            // a report on disk for the operator to inspect.
            Ok(gate(fail_on, &report))
        }
        Command::Decode { header, format } => {
            let value = match header {
                Some(value) => value,
                None => {
                    use std::io::Read;
                    let mut text = String::new();
                    std::io::stdin().read_to_string(&mut text)?;
                    text
                }
            };
            let record = decode_payment_response(&value).context("failed to decode header")?;
            print!("{}", render_records(std::slice::from_ref(&record), format));
            Ok(ExitCode::SUCCESS)
        }
        Command::Sample { format } => {
            let records = sample_records();
            print!("{}", render_records(&records, format));
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Maps the configured policy onto a process exit status.
///
/// Called only after the command's own output has been emitted, so a failing
/// gate never costs the operator the artifact.
fn gate(policy: FailOn, report: &ValidationReport) -> ExitCode {
    if policy.violated(report) {
        eprintln!(
            "gate failed: {} error(s), {} warning(s) at fail-on={}",
            report.errors,
            report.warnings,
            policy.as_str()
        );
        ExitCode::from(EXIT_GATE_FAILED)
    } else {
        ExitCode::SUCCESS
    }
}
