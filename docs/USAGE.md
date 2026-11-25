# Usage

All commands read settlement records from `--input <FILE>` (or stdin when
omitted / set to `-`). Records may be a JSON array, JSONL, or a single object.

## Common workflow

```bash
# Generate a sample ledger to play with.
kite-x402-reconciler sample > ledger.json

# Inspect / normalize.
kite-x402-reconciler ingest --input ledger.json --format csv

# Check validity (CI-friendly: --fail-on error exits 3 on any error).
kite-x402-reconciler validate --input ledger.json --format text --fail-on error

# Money view.
kite-x402-reconciler summary --input ledger.json --format json

# Security view.
kite-x402-reconciler duplicates --input ledger.json

# Two-ledger view: does our ledger agree with the facilitator's?
kite-x402-reconciler reconcile --local ledger.json --facilitator facilitator-export.jsonl

# Everything to one file.
kite-x402-reconciler report --input ledger.json --output report.json

# Human-readable view: one self-contained HTML file you can double-click.
kite-x402-reconciler report --input ledger.json --output report.html --format html
```

### Output formats

`--format` accepts `text`, `json`, `csv` and `html` (defaults vary by command).
`html` produces a single standalone HTML5 document with inlined CSS — no
JavaScript, no external stylesheets, no network calls — so it opens offline by
double-clicking. `report --format html` shows overview cards, per-network /
payer / day tables and a color-coded findings table; `reconcile --format html`
shows the discrepancy tables. All ledger-sourced strings are HTML-escaped, so
a `<script>` in an error message is shown as text rather than executed. The
`--fail-on` exit-code contract is unchanged: a dirty ledger still exits `3`,
and the HTML file is written before the gate is applied.

## Local web UI (optional)

A browser front-end is available behind the non-default `web` feature:

```bash
cargo run --release --features web -- serve
# prints: kite-x402-reconciler web UI: http://127.0.0.1:<port>
```

Open that address. Port `0` (the default) picks a free port; use
`serve --port 8787` to fix one. The page lets you drag in a JSON/JSONL ledger,
load a sample, click into each validation finding and discrepancy, and download
the full HTML report. The two-ledger tab does the same for `reconcile`.

The server binds loopback only and has **no authentication** — do not expose it
to a LAN or a public interface. It makes no outbound requests, and the page has
no external CSS/JS/fonts. Every analysis is performed by the same library
functions the CLI calls; the browser is only a shell. A normal build (no
`--features web`) remains a six-dependency, single-binary CLI.

## Feeding real x402 data

If your service logs the facilitator's `SettleResponse` JSON, point `--input`
at it directly. If you captured the `PAYMENT-RESPONSE` header, decode it:

```bash
kite-x402-reconciler decode '<base64 header>' --format json
```

You can also wrap headers in a JSON file and ingest them in bulk:

```json
[{"paymentResponse": "<base64 header 1>"}, {"paymentResponse": "<base64 header 2>"}]
```

```bash
kite-x402-reconciler ingest --input headers.json
```

## Record fields

| Field | Required | Meaning |
|---|---|---|
| `success` | yes | Settlement succeeded |
| `transaction` | yes | Transaction hash (`0x` + 64 hex) |
| `network` | yes | CAIP-2 network, e.g. `eip155:2424` |
| `amount` | no | Amount in atomic units (decimal string) |
| `payer` | no | Payer address (`0x` + 40 hex) |
| `errorReason` / `errorMessage` | no | Required in spirit when `success=false` |
| `resource` / `timestamp` / `nonce` | no | Bookkeeping for audit / replay detection |

Replay detection compares `(payer, nonce)` after canonicalizing both: surrounding
whitespace is not part of either value, a hex nonce (`0x` + hex digits) is
case-insensitive like every other hex identifier, and an opaque nonce keeps its
case. A blank nonce counts as absent, not as a nonce shared by every record.

## Validation codes

| Code | Severity | Meaning |
|---|---|---|
| `failure-without-reason` | error | `success=false` with no error info |
| `invalid-network` | error | Not a CAIP-2 `namespace:reference` |
| `invalid-amount` | error | Amount is not a non-negative integer |
| `duplicate-transaction` | error | Same `transaction` appears twice |
| `duplicate-nonce` | error | Same `(payer, nonce)` appears twice; both sides are trimmed, and a hex nonce is case-insensitive |
| `missing-transaction` | error | Empty transaction hash |
| `success-with-reason` | warning | `success=true` but an error reason is set |
| `invalid-transaction` | warning | Hash is not `0x` + 64 hex |
| `zero-amount` | warning | Successful settlement with amount 0 |
| `invalid-payer` | warning | Payer is not `0x` + 40 hex |
| `invalid-timestamp` | warning | Timestamp is not RFC 3339 |

## Library usage

```rust
use kite_x402_reconciler::ingest::parse_records;
use kite_x402_reconciler::reconcile::summarize;
use kite_x402_reconciler::validate::validate_all;

let records = parse_records(&std::fs::read_to_string("ledger.json")?)?;
let report = validate_all(&records);
assert!(report.is_ok());
let summary = summarize(&records);
println!("settled atomic units: {}", summary.total.amount_atomic);
# Ok::<(), Box<dyn std::error::Error>>(())
```
