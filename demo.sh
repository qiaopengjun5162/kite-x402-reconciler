#!/usr/bin/env bash
# End-to-end demo of kite-x402-reconciler, suitable as a submission artifact.
# Usage: bash demo.sh   (after `cargo build --release`)
set -euo pipefail

BIN="./target/release/kite-x402-reconciler"
if [[ ! -x "$BIN" ]]; then
  echo "building release binary first..."
  cargo build --release --locked
fi

echo "== 1. generate a sample ledger (contains a deliberate duplicate tx) =="
"$BIN" sample > /tmp/kxr-ledger.json
head -c 300 /tmp/kxr-ledger.json; echo "..."

echo
echo "== 2. validate (non-strict: prints issues) =="
"$BIN" validate --input /tmp/kxr-ledger.json --format text

echo
echo "== 3. summary: exact atomic-unit totals by network/payer/day =="
"$BIN" summary --input /tmp/kxr-ledger.json --format text

echo
echo "== 4. duplicates: replay / double-settle detection =="
"$BIN" duplicates --input /tmp/kxr-ledger.json --format text

echo
echo "== 5. decode a PAYMENT-RESPONSE header =="
HEADER=$(python3 -c 'import base64,json;print(base64.b64encode(json.dumps({"success":True,"transaction":"0x"+"a"*64,"network":"eip155:2424","amount":"999"}).encode()).decode())')
"$BIN" decode "$HEADER" --format text

echo
echo "== 6. full JSON report =="
"$BIN" report --input /tmp/kxr-ledger.json --output /tmp/kxr-report.json --format json
echo "wrote /tmp/kxr-report.json ($(wc -c < /tmp/kxr-report.json) bytes)"

echo
echo "== 7. gate: --fail-on error exits 3, report still written =="
set +e
"$BIN" report --input /tmp/kxr-ledger.json --output /tmp/kxr-gated-report.json --format json --fail-on error
STATUS=$?
set -e
echo "exit status: $STATUS (3 = gate failed)"
echo "artifact on disk anyway: $(wc -c < /tmp/kxr-gated-report.json) bytes"

echo
echo "demo complete."
