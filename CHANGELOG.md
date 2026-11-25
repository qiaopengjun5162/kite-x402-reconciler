# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- No behavior change: `summary` gained an invariant test asserting that each
  breakdown (`by_network`, `by_payer`, `by_day`) adds up to the grand total —
  record counts, settled amounts and unavailable-amount counts alike. The
  existing tests pinned individual groups to expected values; none checked that
  the groups partition the total, so a record counted twice (or dropped from
  one view) could have passed. Verified to fail against the pre-v0.2.2
  raw-value grouping.

## [0.2.3] - 2026-11-25

### Fixed

- `sample_records()` documented "one duplicate transaction and one replayed
  nonce", but the returned ledger only had the duplicate transaction — the two
  records for payer `a` used different nonces, so `duplicates` (the tool's
  security view) never demonstrated replay detection on the sample the docs
  tell users to run. The replaying record now reuses `nonce-1`, so the sample
  exercises both documented findings.
- Three parsing/validity checks still used raw field values while the rest of
  the pipeline trims before comparing: an `errorReason`/`errorMessage` of `""`
  or whitespace passed `failure-without-reason` (and conversely tripped
  `success-with-reason`), a padded amount (`" 1000 "`) was rejected as
  `invalid-amount`, dropped from `summary` totals, and reported as an
  `amount_mismatch` against its unpadded twin, and a padded RFC 3339 timestamp
  was flagged `invalid-timestamp` and fell into the `(unknown)` day bucket.
  All now trim before parsing/judging, consistent with the hash, payer and
  network fields, so a hand-edited export no longer contradicts itself across
  `validate`, `reconcile` and `summary`.

## [0.2.2] - 2026-11-25

### Changed

- Release builds now use thin LTO with a single codegen unit. The distributed
  CLI binary shrank ~16% (1,399,072 → 1,177,632 bytes on macOS) and the release
  build got faster, while default debug symbols are kept so crashes still
  produce a usable backtrace. No runtime behavior or CLI surface changed.

### Fixed

- `validate` and `duplicates` detected repeated transaction hashes using the
  raw string, while `reconcile` normalizes hashes with trim + lowercase before
  pairing. The same hash written once as `0xABCD` and once as ` 0xabcd ` was
  reported as distinct records by validation yet paired as one settlement by
  reconciliation — the duplicate check missed exactly the records that would
  later collide. Both paths now share `model::normalize_hash`, so duplicate
  detection and reconciliation agree on what counts as the same hash.
- Replay (`duplicate-nonce`) detection keyed the nonce by the raw payer string.
  EVM addresses are case-insensitive hex, so the same payer written as `0xABCD`
  and ` 0xabcd ` looked like two payers and a reused nonce escaped detection —
  the exact double-spend case the check exists to catch. The payer is now
  normalized via the shared hex-identity rule (`model::normalize_address`),
  which uses the same canonicalization as hashes, so replay and duplicate
  detection agree on identity.
- `reconcile` paired records within a transaction hash by their row position.
  Since the CAIP-2 network is part of a settlement's identity, a hash seen on
  two chains with both sides agreeing could still be reported as two
  cross-chain mismatches if the two exports listed the rows in a different
  order — the verdict depended on export order, not on the data. Records in a
  hash group are now paired to the first unpaired record on the **same
  network**, falling back to another network only when that is exhausted; a
  genuine cross-chain difference is still reported as `network_mismatch`.
  Unpaired facilitator records keep their input order.
- `validate` checked the CAIP-2 network with the raw field while `reconcile`
  trims networks before comparing. A record whose network was padded (e.g.
  `" eip155:2424 "`) was flagged as `invalid-network` by validation yet paired
  as a same-chain match by reconciliation — the two commands disagreed about
  the same data. `network_id()` now trims surrounding whitespace, matching the
  reconciliation view; whitespace inside the identifier is still rejected.
- The format checks for transaction hashes and payer addresses used the raw
  field, so a valid identifier padded with whitespace (e.g. `"  0xaaaa…  "`)
  was flagged `invalid-transaction` / `invalid-payer` by `validate` while
  `reconcile` and duplicate detection — which trim before comparing — treated
  the same record as clean. `validate --fail-on warning` could therefore reject
  a ledger reconciliation considered valid. Both checks now trim before
  validating, consistent with the rest of the identity pipeline.
- A JSON **array** of `paymentResponse` / `header` envelopes — the bulk header
  import documented in `docs/USAGE.md` — was decoded by deserializing each
  element straight into `SettlementRecord`, skipping envelope decoding
  entirely. Every element failed with `missing field success`, while the same
  envelope as a single object worked. Array elements now go through the same
  `decode_record_value` path as single objects and JSONL lines.
- Input files beginning with a UTF-8 BOM (U+FEFF) failed to parse in every
  format. `str::trim()` does not strip the BOM (it is not Unicode White_Space)
  and serde_json rejects it at column 1. The BOM is now stripped before
  parsing, so Windows/Excel/PowerShell exports ingest like any other file.
- The `summary` breakdowns keyed `by_network` and `by_payer` on the raw export
  spelling, while replay detection, duplicate detection and reconciliation all
  key the same identities through `network_id()` (trim) and `normalize_address`
  (trim + lowercase). A single payer written `0xABCD` in one row and ` 0xabcd `
  in another — or a CAIP-2 network with stray padding — was reported as two
  groups with fragmented totals. Consumers of the JSON/CSV `by_payer` and
  `by_network` sections could therefore undercount a single payer or chain.
  Both breakdowns now group on the same canonical identities as the rest of the
  pipeline.
- Both CSV renders (`summary` and the full `report`) omitted `amount_overflows`,
  which text prints as a WARNING and JSON serializes. A machine CSV consumer
  therefore saw a plausible-but-incomplete `amount_atomic` with no signal that
  unrepresentable amounts were dropped — contradicting the "totals never
  silently lie" guarantee. Both headers now carry a trailing `amount_overflows`
  column; existing columns keep their names and order.

## [0.2.1] - 2026-11-21

### Fixed

- `reconcile` treated a transaction hash as a complete settlement identity, so
  a record logged against `eip155:2366` paired cleanly with the facilitator's
  `eip155:2368` record at the same hash and amount. The payment landed on a
  different chain than the ledger claims — exactly the kind of error this tool
  exists to catch. The CAIP-2 network is now checked before the amount; a
  mismatch produces a new `network_mismatch` finding and counts as an error
  under `--fail-on error`. Network strings are trimmed but not case-folded,
  since CAIP-2 references may be case-sensitive.
- When both the network and the amount differ, `network_mismatch` wins: the two
  records are not the same settlement, so reporting an "amount mismatch" as
  well would imply a single payment with two right answers.

### Added

- `NetworkMismatch` in `diff` and a `network_mismatch` column/section in the
  CSV and text renders. JSON gains the field automatically via `Serialize`.
  The CSV header gains a trailing `facilitator_network` column; existing
  columns keep their names and order.

**Test count: 81 (54 unit + 27 integration).**

## [0.2.0] - 2026-11-20

### Added

- New `reconcile --local <file> --facilitator <file>` command: pairs two
  ledgers on the settlement transaction hash and reports `only_local`,
  `only_facilitator` and `amount_mismatch`. This is the command the tool's name
  always implied — every other command only audits a single ledger. Failed
  settlements are excluded from matching (no on-chain transaction exists to
  pair) and counted as `skipped_failed`; a successful record without a
  transaction hash is a warning, not a silent skip. Supports the same
  `--fail-on` gate and exit code `3` as the other commands.
- `--fail-on none|error|warning` on `validate`, `duplicates`, `reconcile` and
  `report`: turns findings into process exit status `3` so the CLI can gate a
  cron job or CI step. `report` writes its artifact first, so a failing gate
  never costs the operator the report.
- `gate::FailOn` module: the threshold policy lives in one unit-tested place
  instead of being recomputed per command. `FailOn::violated_counts(errors,
  warnings)` extends it to reports that do not build a `ValidationReport`.
- `ValidationReport::from_issues()` constructor: severity counting lives in one
  place instead of being recomputed by hand in the CLI.
- Integration tests pinning the exit-code contract (`0`/`1`/`2`/`3`), the
  report-still-written guarantee, the `--strict` alias, and the rule that an
  unreadable input is `1`, never `3` — a broken input is not a finding.
- Regression tests for JSON report parseability, amount overflow, UTC day
  grouping, single-header CSV reports, and both directions of a ledger diff.
- `tests/cli.rs`: end-to-end integration tests driving the real binary via
  stdin/stdout.
- `docs/TESTING.md`: test commands, coverage matrix, build/deploy and
  troubleshooting.
- `demo.sh`: runnable end-to-end demonstration covering every command
  (submission artifact).

### Changed

- Gate failures now exit `3` instead of `1`. `validate --strict` is kept as a
  hidden alias for `--fail-on error` and follows the same code. `1` remains
  "runtime failure" and `2` remains clap's usage error, so a wrapper can tell a
  dirty ledger from a typo. See "Exit codes" in the README.
- `report --format csv` now emits a single flat table with a leading `section`
  column (`summary` / `issue`) instead of two concatenated CSV tables with
  different headers. All rows share one header and column count.
- `SettlementRecord::amount_atomic()` returns `Result<Option<u128>, _>` instead
  of `Option<Result<u128, _>>` — one layer instead of two. (`pub` API change.)

### Fixed

- `summary` by-day grouping now normalizes timestamps to UTC before taking the
  calendar date. A ledger mixing offsets no longer splits/merges settlement
  days incorrectly.
- CI now really verifies the declared MSRV: a `1.88` job runs `cargo check
  --locked --all-targets`, so a dependency silently raising the floor is caught.
- `report --format json` now emits **one** valid JSON document
  (`{"summary": ..., "validation": ...}`) instead of two concatenated objects
  that no JSON parser could read. New regression test parses the output.
- `u128` amount overflow no longer panics in debug (or silently wraps in
  release). `Totals.amount_overflows` counts dropped amounts and the text
  report prints a prominent warning so totals never silently lie.
- README `decode` quick-start example used a fake base64 payload that always
  failed; replaced with a real, parseable `PAYMENT-RESPONSE` header.

**Test count: 76 (51 unit + 25 integration). Verified on stable and MSRV 1.88.**

## [0.1.0] - 2026-11-17

### Added

- `SettlementRecord` model mirroring the Kite facilitator `SettleResponse`,
  with camelCase JSON aliases, CAIP-2 `NetworkId` validation and `u128`
  atomic-unit amount parsing.
- `ingest`: read records from JSON arrays, JSONL, single objects or stdin;
  decode base64 `PAYMENT-RESPONSE` headers and `paymentResponse` envelopes.
- `validate`: per-record rules (success/reason consistency, network, amount,
  transaction hash, payer, timestamp) plus cross-record duplicate-transaction
  and replayed-nonce detection.
- `reconcile`: exact (`u128`) aggregation by network, payer and day, and
  `format_atomic` for float-free human-readable amounts.
- `report`: text / JSON / CSV rendering (RFC 4180 CSV escaping).
- CLI with seven subcommands: `ingest`, `validate`, `summary`, `duplicates`,
  `report`, `decode`, `sample`; `--strict` mode for CI.
- Documentation: README, DESIGN (five-repo comparison), ARCHITECTURE, USAGE,
  `.env.example`, CHANGELOG, Apache-2.0 license, CI workflow.

[Unreleased]: https://github.com/qiaopengjun5162/kite-x402-reconciler/compare/v0.2.3...HEAD
[0.2.3]: https://github.com/qiaopengjun5162/kite-x402-reconciler/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/qiaopengjun5162/kite-x402-reconciler/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/qiaopengjun5162/kite-x402-reconciler/releases/tag/v0.2.1
[0.2.0]: https://github.com/qiaopengjun5162/kite-x402-reconciler/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/qiaopengjun5162/kite-x402-reconciler/releases/tag/v0.1.0
