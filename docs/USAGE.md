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
```

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

## Validation codes

| Code | Severity | Meaning |
|---|---|---|
| `failure-without-reason` | error | `success=false` with no error info |
| `invalid-network` | error | Not a CAIP-2 `namespace:reference` |
| `invalid-amount` | error | Amount is not a non-negative integer |
| `duplicate-transaction` | error | Same `transaction` appears twice |
| `duplicate-nonce` | error | Same `(payer, nonce)` appears twice |
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
