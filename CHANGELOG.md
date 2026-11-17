# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
