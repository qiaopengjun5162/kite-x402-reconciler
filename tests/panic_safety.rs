//! Panic-safety smoke test: adversarial input must produce `Err`/empty output,
//! never a panic.
//!
//! A reconciler is usually fed exports it did not create — hand-edited CSVs
//! flattened into JSONL, truncated log dumps, a `PAYMENT-RESPONSE` header cut
//! off by a proxy. None of that is allowed to abort a cron job with a panic
//! (exit 101) when it could be reported as a decode error (exit 1). This test
//! walks the whole library surface with pseudo-random bytes so a new `unwrap`,
//! slice index or subtraction underflow shows up as a failing test rather than
//! a production backtrace.

use kite_x402_reconciler::diff::reconcile;
use kite_x402_reconciler::ingest::{decode_payment_response, parse_records};
use kite_x402_reconciler::reconcile::summarize;
use kite_x402_reconciler::report::{
    render_diff, render_full_report, render_records, render_summary, render_validation, Format,
};
use kite_x402_reconciler::validate::validate_all;

/// Deterministic xorshift64* so a failure is reproducible from the seed alone
/// (no `proptest` dependency, no proptest regression-file toolchain).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

/// Alphabet biased toward JSON/base64 syntax so most samples reach the parser
/// rather than dying on the first byte.
const ALPHABET: &[u8] = br#"{}[]",:0123456789abcdefABCDEFxeip15-_. \t\n\r+/=!#%$'"#;

fn random_text(rng: &mut Rng, max_len: usize) -> String {
    let len = rng.below(max_len);
    let mut bytes = Vec::with_capacity(len);
    for _ in 0..len {
        bytes.push(ALPHABET[rng.below(ALPHABET.len())]);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn random_record_json(rng: &mut Rng) -> String {
    // Small pools make duplicates, same-hash-different-amount pairs and
    // cross-chain pairs common; uniformly random 64-hex hashes would almost
    // never collide and the pairing passes would stay untested.
    let hashes = ["0x", "0XAB", "0xab ", "0xCD"];
    let networks = [
        "eip155:2424",
        " eip155:2424 ",
        "eip155:1",
        "eip155:2424:extra",
    ];
    let amounts = [
        "100",
        " 100 ",
        "200",
        "",
        "1.5",
        "0",
        "340282366920938463463374607431768211455",
    ];
    let payers = ["0xAB", "0xab", " 0xCD ", "not-an-address"];
    let nonces = ["n1", " n1 ", "0xAB", "0xab", ""];

    // The required fields are always present, so the record reaches the
    // reconciler instead of dying in serde. Optional fields are omitted at
    // random, because absence is a real shape in flattened exports.
    let mut parts = vec![
        format!("\"success\":{}", rng.below(4) != 0),
        format!("\"transaction\":\"{}\"", hashes[rng.below(hashes.len())]),
        format!("\"network\":\"{}\"", networks[rng.below(networks.len())]),
    ];
    if rng.below(3) != 0 {
        parts.push(format!(
            "\"amount\":\"{}\"",
            amounts[rng.below(amounts.len())]
        ));
    }
    if rng.below(3) != 0 {
        parts.push(format!("\"payer\":\"{}\"", payers[rng.below(payers.len())]));
    }
    if rng.below(3) != 0 {
        parts.push(format!("\"nonce\":\"{}\"", nonces[rng.below(nonces.len())]));
    }
    if rng.below(3) == 0 {
        // Failure paths need a reason, or every failed record is also reported
        // as failure-without-reason and drowns the other findings.
        parts.push(format!(
            "\"errorReason\":\"{}\"",
            ["expired", "  ", ""][rng.below(3)]
        ));
    }
    if rng.below(4) == 0 {
        parts.push(format!(
            "\"timestamp\":\"{}\"",
            [
                "2026-11-25T10:00:00Z",
                " 2026-11-25T23:30:00-05:00 ",
                "not-a-time"
            ][rng.below(3)]
        ));
    }
    format!("{{{}}}", parts.join(","))
}

fn random_ledger_json(rng: &mut Rng) -> String {
    match rng.below(3) {
        0 => format!("[{}]", random_record_json(rng)),
        1 => {
            let count = 1 + rng.below(4);
            let rows: Vec<_> = (0..count).map(|_| random_record_json(rng)).collect();
            format!("[{}]", rows.join(","))
        }
        _ => {
            let count = 1 + rng.below(4);
            (0..count)
                .map(|_| random_record_json(rng))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

#[test]
fn adversarial_ledgers_never_panic_and_reconcile_stays_consistent() {
    let mut rng = Rng(0x5eed_1234_abcd_9876);
    let formats = [Format::Text, Format::Json, Format::Csv];
    // CI runs the default (a smoke test); a fuzzing session can raise it, e.g.
    // `PANIC_SAFETY_ITERATIONS=300000 cargo test --release --test panic_safety`.
    let iterations: usize = std::env::var("PANIC_SAFETY_ITERATIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2_000);

    let mut parsed_ledgers = 0usize;
    let mut informative_diffs = 0usize;
    let mut paired = 0usize;
    for iteration in 0..iterations {
        let text = random_text(&mut rng, 160);

        // 1. Any byte soup through the record parser and the header decoder.
        if let Ok(records) = parse_records(&text) {
            let report = validate_all(&records);
            let summary = summarize(&records);
            for format in formats {
                // Rendering must also survive whatever parsing accepted, and
                // every format must be produced without an index panic.
                let _ = render_records(&records, format);
                let _ = render_validation(&report, format);
                let _ = render_summary(&summary, format);
                let _ = render_full_report(&summary, &report, format);
            }
        }
        let _ = decode_payment_response(&text);

        // 2. Structured-but-wrong records: the shape a flattening script
        //    produces when a column is missing or carries junk.
        let ledger = random_ledger_json(&mut rng);
        if let Ok(records) = parse_records(&ledger) {
            parsed_ledgers += 1;
            let report = validate_all(&records);
            let summary = summarize(&records);
            let _ = render_full_report(&summary, &report, Format::Csv);
            assert_eq!(
                report.errors + report.warnings,
                report.issues.len(),
                "iteration {iteration}: severity counters must match the issue list"
            );
            assert_eq!(
                summary.total.count,
                records.len(),
                "iteration {iteration}: every record must be totalled"
            );
        }

        // 3. Two independent ledgers through the pairing pass, including the
        //    cross-chain fallback and the one-sided branches.
        if let (Ok(local), Ok(facilitator)) = (
            parse_records(&random_ledger_json(&mut rng)),
            parse_records(&random_ledger_json(&mut rng)),
        ) {
            let diff = reconcile(&local, &facilitator);
            paired += 1;
            if diff.matched
                + diff.amount_mismatch.len()
                + diff.network_mismatch.len()
                + diff.only_local.len()
                + diff.only_facilitator.len()
                > 0
            {
                informative_diffs += 1;
            }
            for format in formats {
                let _ = render_diff(&diff, format);
            }

            // Swapping the two sides must only mirror the one-sided findings:
            // the verdict cannot depend on which export is called "local".
            let mirrored = reconcile(&facilitator, &local);
            assert_eq!(
                (diff.matched, diff.skipped_failed, diff.skipped_unidentified),
                (
                    mirrored.matched,
                    mirrored.skipped_failed,
                    mirrored.skipped_unidentified
                ),
                "iteration {iteration}: matched/skipped must not depend on side order"
            );
            assert_eq!(
                (diff.amount_mismatch.len(), diff.network_mismatch.len()),
                (
                    mirrored.amount_mismatch.len(),
                    mirrored.network_mismatch.len()
                ),
                "iteration {iteration}: mismatch counts must not depend on side order"
            );
            assert_eq!(
                (diff.only_local.len(), diff.only_facilitator.len()),
                (mirrored.only_facilitator.len(), mirrored.only_local.len()),
                "iteration {iteration}: one-sided findings must mirror when the sides swap"
            );
            assert_eq!(
                diff.errors(),
                mirrored.errors(),
                "iteration {iteration}: the gate verdict must not depend on side order"
            );

            // Conservation: pairing is a bijection, so every identifiable
            // record on one side must land in exactly one bucket. A pairing
            // pass that consumes a counterparty twice, or drops a leftover,
            // would break this while still looking plausible per bucket.
            let identifiable = |records: &[kite_x402_reconciler::model::SettlementRecord]| {
                records
                    .iter()
                    .filter(|record| record.success && !record.transaction.trim().is_empty())
                    .count()
            };
            let paired = diff.matched + diff.amount_mismatch.len() + diff.network_mismatch.len();
            assert_eq!(
                identifiable(&local),
                paired + diff.only_local.len(),
                "iteration {iteration}: local records must be fully accounted for"
            );
            assert_eq!(
                identifiable(&facilitator),
                paired + diff.only_facilitator.len(),
                "iteration {iteration}: facilitator records must be fully accounted for"
            );
        }
    }
    // The corpus must not silently decay into testing only the decode-error
    // path: `paired` and `informative_diffs` prove the assertions above ran
    // against real ledgers, not empty vectors.
    assert!(
        parsed_ledgers > iterations / 2,
        "the generator must produce parseable ledgers, not just junk"
    );
    assert!(paired > iterations / 4, "the corpus must exercise pairing");
    assert!(
        informative_diffs > paired / 2,
        "the corpus must produce real findings, not only empty diffs"
    );
}
