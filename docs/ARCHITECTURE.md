# Architecture

## Layering

```
src/main.rs        clap CLI — thin, only wires commands to the library
src/lib.rs         public library API + sample_records()
  ├─ model.rs      SettlementRecord (mirrors Kite SettleResponse), NetworkId
  ├─ errors.rs     ReconcileError + Result alias
  ├─ ingest.rs     text/file/stdin -> Vec<SettlementRecord>; decode PAYMENT-RESPONSE
  ├─ validate.rs   per-record rules + cross-record duplicate/replay detection
  ├─ reconcile.rs  aggregate by network/payer/day with exact u128 arithmetic
  ├─ diff.rs       local-vs-facilitator pairing; four discrepancy kinds
  ├─ gate.rs       --fail-on threshold -> process exit status
  └─ report.rs     render ValidationReport / Summary / diff / records as text|json|csv|html
```

The CLI deliberately contains no business logic. Everything is testable through the library API.

## Data flow

```
file / stdin ──ingest::parse_records──> Vec<SettlementRecord>
                                            │
                    ┌───────────────────────┼────────────────────────┐
                    v                       v                        v
             validate::validate_all   reconcile::summarize      report::render_records
                    │                       │
              ValidationReport          Summary
                    └────────report.rs render text|json|csv|html─────┘
                                       │
                              gate.rs --fail-on -> exit 0/1/2/3
```

Two-ledger mode feeds a second file into `diff::reconcile`, which emits a
`LedgerDiff` rendered by the same `report.rs` and gated by the same `gate.rs`.

## Key design decisions

1. **Facilitator-compatible model.** Field names and JSON aliases match
   `kite-x402-axum`'s `SettleResponse`, so records exported by a real Kite
   service can be fed in unchanged. Bookkeeping fields (resource / timestamp /
   nonce) are optional so real facilitator output still deserializes.

2. **Exact money math.** `amount` is a decimal string in atomic units and is
   parsed to `u128`. Sums are `u128`. There is no floating point anywhere in
   the money path. `format_atomic(value, decimals)` converts to a human string
   with string padding only when display is needed.

3. **CAIP-2, not an allowlist.** Network validation checks the `namespace:reference`
   shape instead of hard-coding chain IDs, so new Kite networks work without a
   code change.

4. **Detecting replay.** A transaction hash should appear once (duplicate =
   error). A `(payer, nonce)` pair should appear once (reused nonce = error);
   the same nonce under a different payer is fine.

5. **Offline-first.** No network calls; all deps are common crates with local
   caches. `cargo test --offline` works after the first fetch.

## Extension points

- Add a new validation rule: add a check in `validate::validate_all` plus a
  stable `code` string and a unit test.
- Add a new grouping: add a `BTreeMap` to `Summary` and fill it in `summarize`.
- Add an input format: extend `ingest::parse_records`.
- Add an output format: extend `report::Format` and the render functions.
