# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed

- `report --format json` now emits **one** valid JSON document
  (`{"summary": ..., "validation": ...}`) instead of two concatenated objects
  that no JSON parser could read. New regression test parses the output.
- `u128` amount overflow no longer panics in debug (or silently wraps in
  release). `Totals.amount_overflows` counts dropped amounts and the text
  report prints a prominent warning so totals never silently lie.
- README `decode` quick-start example used a fake base64 payload that always
  failed; replaced with a real, parseable `PAYMENT-RESPONSE` header.

### Added

- `tests/cli.rs`: 12 end-to-end integration tests driving the real binary via
  stdin/stdout (help, sample, validate strict pass/fail, summary, duplicates,
  decode, CSV pipeline, full report) plus library boundary tests.
- `docs/TESTING.md`: test commands, coverage matrix, build/deploy and
  troubleshooting.
- `demo.sh`: runnable end-to-end demonstration covering all six commands
  (submission artifact).
- README: Configuration and Known limitations sections.

- Regression tests for JSON report parseability and amount overflow.

**Test count: 47 (34 unit + 13 integration).**

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

[Unreleased]: https://github.com/qiaopengjun5162/kite-x402-reconciler/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/qiaopengjun5162/kite-x402-reconciler/releases/tag/v0.1.0
