# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/qiaopengjun5162/kite-x402-reconciler/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/qiaopengjun5162/kite-x402-reconciler/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/qiaopengjun5162/kite-x402-reconciler/releases/tag/v0.1.0
