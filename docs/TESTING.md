# Testing & Deployment

## Test commands

```bash
# All tests (unit + integration + doc tests)
cargo test

# With the checked-in lockfile, fully offline after first fetch
cargo test --locked --offline

# Only the CLI end-to-end integration tests
cargo test --test cli

# Static checks required by the project's quality gate
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

## Coverage scenarios

| Layer | What is covered |
|---|---|
| `model` (unit) | CAIP-2 valid/invalid forms, namespace/reference split, facilitator camelCase aliases, record round-trip, `u128` amount parse (missing/invalid) |
| `ingest` (unit) | JSON array / single object / JSONL, blank lines, empty input, malformed input, base64 `PAYMENT-RESPONSE`, `paymentResponse` envelope, file input |
| `validate` (unit) | success/reason consistency, network, amount, tx hash, payer, timestamp, duplicate transaction, replayed nonce (same vs different payer), empty batch |
| `reconcile` (unit) | totals and group-by, failed/invalid amounts excluded, `format_atomic` exactness and boundaries |
| `report` (unit) | RFC 4180 CSV escaping, format parsing, validation/summary/record rendering in text/JSON/CSV |
| `tests/cli.rs` (integration) | `--help`, `sample` JSON, `validate --strict` exit codes (pass + fail), `summary` via stdin, duplicate nonce detection, header decode, CSV pipeline, full report |

## Build & deploy

This is a standalone CLI with no runtime service. To deploy:

```bash
cargo build --release --locked
# Copy the single binary to the target host:
cp target/release/kite-x402-reconciler /usr/local/bin/
kite-x402-reconciler --version
```

CI (`.github/workflows/ci.yml`) runs fmt, clippy (`-D warnings`), tests and a
release build on every push/PR to `main`/`master`.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `--strict` exits 1 with no visible issue on stdout | Issues are printed to stdout; check the terminal output above the shell prompt, or drop `--strict` to see them and still exit 0. |
| `invalid-amount` on a value like `1.5` | `amount` must be an integer in atomic units (string), e.g. `1500000` for 1.5 tokens at 6 decimals. |
| `invalid-network` on `eip155:2424` | Not expected — that is valid. Ensure there is no whitespace or a third `:`. |
| `decode` says "invalid base64" | The header must be raw base64 of a JSON object, without the `PAYMENT-RESPONSE:` label; trim quotes. |
| `cargo test --locked` fails "cannot create lock file" | The lock file is checked in; if missing, run `cargo test --offline` once to generate `Cargo.lock`. |
| `git push` times out on port 443 | Use the local proxy: `git -c http.proxy=http://127.0.0.1:7890 -c https.proxy=http://127.0.0.1:7890 push`. |
| No `tests/` discovered | Ensure you run from the crate root (`kite-x402-reconciler/`), not the parent `Rust/` dir. |
