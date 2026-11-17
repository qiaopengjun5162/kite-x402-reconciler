//! CLI for `kite-x402-reconciler`.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use kite_x402_reconciler::ingest::{decode_payment_response, read_records};
use kite_x402_reconciler::reconcile::summarize;
use kite_x402_reconciler::report::{
    render_full_report, render_records, render_summary, render_validation, Format,
};
use kite_x402_reconciler::sample_records;
use kite_x402_reconciler::validate::validate_all;

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
        /// Exit non-zero when any error-severity issue is found.
        #[arg(long)]
        strict: bool,
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Ingest { input, format } => {
            let records = read_records(input.as_deref()).context("failed to read records")?;
            print!("{}", render_records(&records, format));
        }
        Command::Validate {
            input,
            format,
            strict,
        } => {
            let records = read_records(input.as_deref()).context("failed to read records")?;
            let report = validate_all(&records);
            print!("{}", render_validation(&report, format));
            if strict && !report.is_ok() {
                bail!("validation found {} error(s)", report.errors);
            }
        }
        Command::Summary { input, format } => {
            let records = read_records(input.as_deref()).context("failed to read records")?;
            let summary = summarize(&records);
            print!("{}", render_summary(&summary, format));
        }
        Command::Duplicates { input, format } => {
            let records = read_records(input.as_deref()).context("failed to read records")?;
            let report = validate_all(&records);
            let duplicates: Vec<_> = report
                .issues
                .into_iter()
                .filter(|issue| matches!(issue.code, "duplicate-transaction" | "duplicate-nonce"))
                .collect();
            let filtered = kite_x402_reconciler::validate::ValidationReport {
                checked: records.len(),
                errors: duplicates
                    .iter()
                    .filter(|i| i.severity == kite_x402_reconciler::validate::Severity::Error)
                    .count(),
                warnings: 0,
                issues: duplicates,
            };
            print!("{}", render_validation(&filtered, format));
        }
        Command::Report {
            input,
            output,
            format,
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
        }
        Command::Sample { format } => {
            let records = sample_records();
            print!("{}", render_records(&records, format));
        }
    }
    Ok(())
}
