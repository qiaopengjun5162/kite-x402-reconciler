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

# 2. Validate it — exits non-zero in --strict mode if any error is found.
kite-x402-reconciler validate --input ledger.json --strict

# 3. Aggregate totals by network / payer / day.
kite-x402-reconciler summary --input ledger.json

# 4. Find duplicate transactions and replayed nonces.
kite-x402-reconciler duplicates --input ledger.json

# 5. Write a full JSON report.
kite-x402-reconciler report --input ledger.json --output report.json --format json

# 6. Decode a PAYMENT-RESPONSE header from a real x402 response.
kite-x402-reconciler decode 'eyJzdWNjZXNzIjp0cnVlLC4uLn0='
```

Input can be a JSON array, JSONL (one object per line), or a single object. Use `--input -` (or omit it) to read from stdin, so it pipes naturally:

```bash
cat ledger.jsonl | kite-x402-reconciler summary --format json
```

## Commands

| Command | What it does |
|---|---|
| `ingest` | Read and normalize records, re-emit as JSON/CSV/text |
| `validate` | Report structural + semantic issues (`--strict` for CI) |
| `summary` | Aggregate by network / payer / day (exact `u128`) |
| `duplicates` | Duplicate transactions + replayed nonces |
| `report` | Summary + issues to a file (or stdout) |
| `decode` | Decode a base64 `PAYMENT-RESPONSE` header |
| `sample` | Print a sample ledger for demos/tests |

Every command accepts `--format text|json|csv` (defaults vary by command).

## Docs

- [Design & five-repo comparison](docs/DESIGN.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Usage](docs/USAGE.md)
- [Changelog](CHANGELOG.md)

## Quality

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

## License

Apache-2.0. See [LICENSE](LICENSE).
