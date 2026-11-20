# kite-x402-reconciler

A small, dependency-light CLI + Rust library for **reconciling and auditing Kite x402 settlement records**.

If you run an x402-paid service on the Kite chain, the facilitator returns a `SettleResponse` for every settled request (also base64-encoded into the `PAYMENT-RESPONSE` header). This tool answers the questions that come *after* the money is collected:

- **Are these records valid?** — field checks, CAIP-2 network, atomic-unit amounts, duplicate transactions, replayed nonces.
- **How much did I earn?** — exact aggregation by network / payer / day, using `u128` (no float errors).
- **Am I being double-charged or replayed?** — duplicate `transaction` and reused `nonce` detection.

This is **not** another payment wrapper. It is the audit/tooling layer that the existing x402 repos do not cover.

## Why this is a real Kite x402 project

- The `SettlementRecord` data model mirrors the Kite facilitator's `POST /settle` response field-for-field (`success`, `errorReason`, `errorMessage`, `payer`, `transaction`, `network`, `amount`, with camelCase aliases).
- `decode` understands the real x402 `PAYMENT-RESPONSE` header (base64 of JSON).
- `network` is parsed and validated as a CAIP-2 identifier (e.g. `eip155:2424`).
- `amount` is treated as an on-chain atomic unit (`u128`), never a float.

See [docs/DESIGN.md](docs/DESIGN.md) for the five-reference-repo comparison and the design rationale.

## Install

```bash
git clone https://github.com/qiaopengjun5162/kite-x402-reconciler.git
cd kite-x402-reconciler
cargo build --release
# binary: target/release/kite-x402-reconciler
```

Requires Rust 1.88+ (edition 2021).

## Quick start

```bash
# 1. Generate a sample ledger (includes a deliberate duplicate transaction).
kite-x402-reconciler sample --format json > ledger.json

# 2. Validate it — add --fail-on error to turn findings into a non-zero exit.
kite-x402-reconciler validate --input ledger.json --fail-on error

# 3. Aggregate totals by network / payer / day.
kite-x402-reconciler summary --input ledger.json

# 4. Find duplicate transactions and replayed nonces.
kite-x402-reconciler duplicates --input ledger.json

# 5. Write a full JSON report (and still exit 3 if the ledger is dirty).
kite-x402-reconciler report --input ledger.json --output report.json --format json --fail-on error

# 6. Decode a PAYMENT-RESPONSE header from a real x402 response.
kite-x402-reconciler decode 'eyJzdWNjZXNzIjp0cnVlLCJ0cmFuc2FjdGlvbiI6IjB4YTFiMmMzZDRhMWIyYzNkNGExYjJjM2Q0YTFiMmMzZDRhMWIyYzNkNGExYjJjM2Q0YTFiMmMzZDRhMWIyYzNkNCIsIm5ldHdvcmsiOiJlaXAxNTU6MjQyNCIsImFtb3VudCI6IjE1MDAwMDAiLCJwYXllciI6IjB4OWY4ZTdkNmM1YjRhMzkyODE3MDY5ZjhlN2Q2YzViNGEzOTI4MTcwNiJ9'
```

Input can be a JSON array, JSONL (one object per line), or a single object. Use `--input -` (or omit it) to read from stdin, so it pipes naturally:

```bash
cat ledger.jsonl | kite-x402-reconciler summary --format json
```

## Commands

| Command | What it does |
|---|---|
| `ingest` | Read and normalize records, re-emit as JSON/CSV/text |
| `validate` | Report structural + semantic issues |
| `summary` | Aggregate by network / payer / day (exact `u128`); days are UTC calendar dates |
| `duplicates` | Duplicate transactions + replayed nonces |
| `reconcile` | Compare a local ledger against the facilitator's, matched on transaction hash |
| `report` | Summary + issues to a file (or stdout) |
| `decode` | Decode a base64 `PAYMENT-RESPONSE` header |
| `sample` | Print a sample ledger for demos/tests |

Every command accepts `--format text|json|csv` (defaults vary by command).
`validate`, `duplicates`, `reconcile` and `report` also accept
`--fail-on none|error|warning` (default `none`).

## Two-ledger reconciliation

`summary` and `duplicates` audit **one** ledger — they answer "is this ledger
self-consistent". They cannot answer "does it agree with what the facilitator
actually settled", and that is the question that costs money. `reconcile` takes
two ledgers and pairs them on the settlement transaction hash:

```bash
kite-x402-reconciler reconcile \
  --local /var/lib/x402/our-records.jsonl \
  --facilitator /var/lib/x402/facilitator-export.jsonl \
  --fail-on error
```

Three kinds of finding come out, and each has a different operational meaning:

| Finding | What it means | Why it is an error |
|---|---|---|
| `only_local` | Your service recorded a settlement the facilitator has no record of | You are billing/delivering against revenue that does not exist |
| `only_facilitator` | The facilitator settled something your service never recorded | The buyer paid for a request you may not have served, and a retry charges them twice |
| `amount_mismatch` | Both sides saw the same transaction but disagree on the amount | Your accounting and the chain disagree |

Matching details worth knowing before you trust the output:

- Hashes are compared after trimming whitespace and lowercasing, because hex
  reaches the tool from hand-edited exports and mixed-case logs.
- **Failed settlements are excluded** from matching and reported as
  `skipped_failed`. A failed `SettleResponse` has no on-chain transaction, so
  pairing it against the other side's silence would report every failure as two
  discrepancies. This is the single most important rule: a naive diff of a
  production ledger is mostly failures.
- A **successful** record with an empty transaction hash is different — it
  claims a settlement that cannot be identified. That is a warning
  (`skipped_unidentified`), not a silent skip, and it is the only thing
  `--fail-on warning` catches that `--fail-on error` does not.
- Duplicate transactions pair in input order; the surplus copy on the longer
  side becomes a one-sided finding instead of disappearing.


## Exit codes

The tool is meant to run unattended, so the status codes are a contract:

| Code | Meaning |
|---|---|
| `0` | Ran, and the ledger met the requested threshold |
| `1` | Runtime failure (unreadable input, malformed ledger, write error) |
| `2` | Usage error (unknown flag or bad value) |
| `3` | Gate failed: findings at or above `--fail-on` |

`report` writes the artifact **before** applying the gate, so a failing run
still leaves a report on disk:

```bash
# Cron/CI: always produce the report, alert only on a dirty ledger.
kite-x402-reconciler report --input /var/lib/x402/today.jsonl \
  --output /var/lib/x402/reports/today.json --fail-on error
case $? in
  0) echo "clean" ;;
  3) echo "ledger needs review" ; exit 1 ;;
  *) echo "reconciler broke" ; exit 1 ;;
esac
```

`validate --strict` is the deprecated spelling of `--fail-on error`. It can only
tighten the threshold, never relax it.

## Configuration

No environment variables are required — the tool is fully offline and every
command takes its input via `--input` (file or stdin). A [`.env.example`](.env.example)
is included for wrapper scripts and documents the optional hints
(`KITE_NETWORK`, `KITE_TOKEN_DECIMALS`, `KITE_RECONCILER_INPUT/OUTPUT`).

## Known limitations

- The tool is **offline only**: it reconciles records you provide. It does not
  call a Kite facilitator RPC or fetch on-chain state itself. `reconcile` closes
  the gap between *your* records and *the facilitator's* records, but you still
  supply both exports.
- `network` validation checks the CAIP-2 **shape** (`namespace:reference`), not
  a chain-id allowlist. A syntactically valid but non-existent chain id will
  pass the check by design (so new Kite networks work without code changes).
- `by_day` grouping converts each RFC 3339 timestamp to **UTC** first, then
  takes the calendar date. A settlement at `23:30-05:00` counts as the next
  UTC day; all payers are judged against the same clock. Without a timestamp,
  the record falls into `(unknown)`.
- Cross-record replay detection is scoped to the **current input batch**.
  Detecting nonces reused across days requires concatenating those ledgers
  into one input.
- Amounts are summed in **atomic units** as `u128`. Values above `u128::MAX`
  are rejected at parse time. Human-readable decimals are a display concern
  (`format_atomic`); the CLI prints atomic units and does not assume a token's
  decimals.
- Some bookkeeping fields (`resource`, `timestamp`, `nonce`) are optional and
  are not part of the facilitator's `SettleResponse`. When absent, day
  grouping falls back to `(unknown)` and nonce replay detection is skipped.
- `validate` flags a hash that isn't `0x` + 64 hex as a **warning**, not an
  error, to stay compatible with test/mock facilitators that use short ids.

## Docs

- [Design & five-repo comparison](docs/DESIGN.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Usage](docs/USAGE.md)
- [Testing & deployment](docs/TESTING.md)
- [Changelog](CHANGELOG.md)

## Quality

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

## License

Apache-2.0. See [LICENSE](LICENSE).
