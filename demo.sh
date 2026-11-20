#!/usr/bin/env bash
# End-to-end demo of kite-x402-reconciler, suitable as a submission artifact.
# Usage: bash demo.sh   (after `cargo build --release`)
set -euo pipefail

BIN="./target/release/kite-x402-reconciler"
# Always build. Cargo is incremental, so this is nearly free when nothing
# changed — and it removes the trap where a stale binary quietly demos the
# previous version's behaviour.
cargo build --release --locked

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
echo "== 7. reconcile: our ledger vs the facilitator's export =="
# Build a plausible facilitator export from the sample: drop the double-counted
# transaction, change one amount, add a settlement we never recorded.
python3 - /tmp/kxr-ledger.json > /tmp/kxr-facilitator.json <<'PY'
import json, sys
records = json.load(open(sys.argv[1]))
seen, out = set(), []
for record in records:
    if not record.get("success"):
        continue                       # a failed settlement has no on-chain tx
    tx = record["transaction"]
    if tx in seen:
        continue                       # local double-counted this one
    seen.add(tx)
    if len(out) == 1:
        record["amount"] = "400000"    # facilitator reports a different amount
    out.append(record)
out.append({"success": True, "transaction": "0x" + "f" * 64,
            "network": "eip155:2424", "amount": "900000"})
print(json.dumps(out, indent=2))
PY
"$BIN" reconcile --local /tmp/kxr-ledger.json --facilitator /tmp/kxr-facilitator.json

echo
echo "== 8. gate: --fail-on error exits 3, report still written =="
set +e
"$BIN" report --input /tmp/kxr-ledger.json --output /tmp/kxr-gated-report.json --format json --fail-on error
STATUS=$?
set -e
echo "exit status: $STATUS (3 = gate failed)"
echo "artifact on disk anyway: $(wc -c < /tmp/kxr-gated-report.json) bytes"

echo
echo "demo complete."
