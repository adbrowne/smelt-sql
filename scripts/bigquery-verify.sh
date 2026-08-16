#!/usr/bin/env bash
# bigquery-verify.sh — prove the provisioned setup actually works.
#
#     bash scripts/bigquery-verify.sh
#
# Checks the encrypted key and config exist, mints a 1-hour token (prompts for
# the passphrase), then runs a probe query and a write/read round-trip against
# the test dataset using ONLY the service account's token — no ambient
# credentials, exactly as the adapter will.
#
# Also times the queries: that per-query latency is the number Phase 1 needs to
# size the generative conformance suite.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
BQ_CONFIG_DIR="${SMELT_BQ_CONFIG_DIR:-$HOME/.config/gcloud-smelt-bq}"

ok()   { printf '  \033[32m✓\033[0m %s\n' "$1"; }
bad()  { printf '  \033[31m✗\033[0m %s\n' "$1"; }

echo "=== Files"
[[ -f "$BQ_CONFIG_DIR/sa-key.json.gpg" ]] && ok "encrypted key present" || { bad "no encrypted key — run scripts/bigquery-key.sh"; exit 1; }
[[ -f "$BQ_CONFIG_DIR/config.env" ]]      && ok "config.env present"     || { bad "no config.env — run scripts/bigquery-key.sh"; exit 1; }

perms=$(stat -c '%a' "$BQ_CONFIG_DIR/sa-key.json.gpg")
[[ "$perms" == "600" ]] && ok "key permissions are 600" || bad "key permissions are $perms (expected 600)"

echo
echo "=== Mint token (passphrase prompt follows)"
bash "$REPO_ROOT/scripts/bigquery-auth.sh"

# shellcheck disable=SC1091
source "$REPO_ROOT/scripts/bigquery-env.sh"

[[ -n "${SMELT_BQ_ACCESS_TOKEN:-}" ]] || { bad "no token exported"; exit 1; }
ok "token exported"

api="https://bigquery.googleapis.com/bigquery/v2/projects/${SMELT_BQ_PROJECT}/queries"
run_q() {
  curl -sS -X POST "$api" \
    -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"query\":$(jq -Rs . <<<"$1"),\"useLegacySql\":false}"
}

echo
echo "=== Probe query (service-account token only)"
start=$(date +%s%N)
resp=$(run_q "SELECT 1 AS ok")
elapsed=$(( ($(date +%s%N) - start) / 1000000 ))
if jq -e '.rows[0].f[0].v == "1"' <<<"$resp" >/dev/null 2>&1; then
  ok "SELECT 1 succeeded in ${elapsed}ms"
else
  bad "probe failed:"; jq -r '.error.message // .' <<<"$resp"; exit 1
fi

echo
echo "=== Write / read round-trip in ${SMELT_BQ_DATASET}"
tbl="verify_$$"
start=$(date +%s%N)
resp=$(run_q "CREATE OR REPLACE TABLE \`${SMELT_BQ_PROJECT}.${SMELT_BQ_DATASET}.${tbl}\` AS SELECT 42 AS answer")
ddl_ms=$(( ($(date +%s%N) - start) / 1000000 ))
if jq -e '.error' <<<"$resp" >/dev/null 2>&1; then
  bad "CREATE failed:"; jq -r '.error.message' <<<"$resp"; exit 1
fi
ok "CREATE OR REPLACE TABLE succeeded in ${ddl_ms}ms"

resp=$(run_q "SELECT answer FROM \`${SMELT_BQ_PROJECT}.${SMELT_BQ_DATASET}.${tbl}\`")
if jq -e '.rows[0].f[0].v == "42"' <<<"$resp" >/dev/null 2>&1; then
  ok "read back the written row"
else
  bad "read-back failed:"; jq -r '.error.message // .' <<<"$resp"; exit 1
fi

run_q "DROP TABLE \`${SMELT_BQ_PROJECT}.${SMELT_BQ_DATASET}.${tbl}\`" >/dev/null
ok "cleaned up (and the dataset's 24h expiry would have caught it anyway)"

echo
echo "=== Least-privilege check: writing to an EXISTING un-granted dataset must be DENIED"
# The control dataset must exist, or a "not found" error would masquerade as a
# permission denial and this check would pass without testing anything.
ctl="${SMELT_BQ_DATASET}_notgranted"
resp=$(run_q "SELECT 1 FROM \`${SMELT_BQ_PROJECT}.${ctl}.__probe__\`")
msg=$(jq -r '.error.message // ""' <<<"$resp")
if grep -qi 'not found: dataset' <<<"$msg"; then
  bad "control dataset ${ctl} is missing — re-run scripts/bigquery-provision.sh."
  bad "Without it this check cannot distinguish 'denied' from 'does not exist'."
  exit 1
fi

resp=$(run_q "CREATE TABLE \`${SMELT_BQ_PROJECT}.${ctl}.nope\` AS SELECT 1")
msg=$(jq -r '.error.message // ""' <<<"$resp")
if grep -qiE 'access denied|permission' <<<"$msg"; then
  ok "correctly DENIED: $(head -c 90 <<<"$msg")…"
elif [[ -z "$msg" ]]; then
  bad "UNEXPECTED: the service account wrote to ${ctl}, which it was never granted."
  bad "The IAM grant is broader than intended — investigate before building on it."
  run_q "DROP TABLE \`${SMELT_BQ_PROJECT}.${ctl}.nope\`" >/dev/null 2>&1 || true
  exit 1
else
  bad "refused, but not for an access reason — verify by hand: $(head -c 120 <<<"$msg")"
  exit 1
fi

echo
echo "Setup verified. Per-query latency above is what sizes the conformance suite."
