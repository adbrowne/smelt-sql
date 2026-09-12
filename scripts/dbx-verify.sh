#!/usr/bin/env bash
# dbx-verify.sh — prove the provisioned Databricks dogfood setup is reachable
# AND correctly scoped.
#
#     bash scripts/dbx-verify.sh
#
# Two legs, both must pass for this script to exit 0:
#
#   1. Reachability — SELECT 1 and SHOW TABLES against BOTH dogfood schemas
#      (smelt_dogfood, smelt_dogfood_oracle) must succeed.
#   2. Refusal — a CREATE TABLE outside the two granted schemas
#      (workspace.default) must FAIL. This leg's exit-status handling is
#      INVERTED from the reachability leg: the probe command succeeding is
#      the failure condition here, so a credential scoped too broadly is
#      caught rather than silently passing
#      (docs/outcomes/20260912-databricks-dogfood-spine/phases/04a-plan.md).
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
CATALOG="${SMELT_DBX_CATALOG:-workspace}"
SCHEMA="${SMELT_DBX_SCHEMA:-smelt_dogfood}"
ORACLE_SCHEMA="${SMELT_DBX_ORACLE_SCHEMA:-smelt_dogfood_oracle}"

ok()  { printf '  \033[32m✓\033[0m %s\n' "$1"; }
bad() { printf '  \033[31m✗\033[0m %s\n' "$1"; }

query() {
  bash "${REPO_ROOT}/scripts/dbx-query.sh" "$1"
}

FAILED=0

echo "=== Reachability"
for schema in "$SCHEMA" "$ORACLE_SCHEMA"; do
  if query "SELECT 1 AS ok" >/dev/null; then
    ok "SELECT 1 reachable"
  else
    bad "SELECT 1 failed"
    FAILED=1
  fi
  if query "SHOW TABLES IN ${CATALOG}.${schema}" >/dev/null; then
    ok "SHOW TABLES IN ${CATALOG}.${schema} succeeded"
  else
    bad "SHOW TABLES IN ${CATALOG}.${schema} failed — is the schema granted?"
    FAILED=1
  fi
done

echo
echo "=== Out-of-scope write refusal"
PROBE_TABLE="smelt_probe_$$"
# Inverted exit-status handling: success here is the FAILURE condition — the
# credential must NOT be able to write outside the two granted schemas.
if query "CREATE TABLE ${CATALOG}.default.${PROBE_TABLE} AS SELECT 1" >/dev/null 2>&1; then
  bad "UNEXPECTED: wrote to ${CATALOG}.default.${PROBE_TABLE} — the credential is scoped too broadly"
  query "DROP TABLE IF EXISTS ${CATALOG}.default.${PROBE_TABLE}" >/dev/null 2>&1 || true
  FAILED=1
else
  ok "write to ${CATALOG}.default correctly refused"
fi

echo
if [[ "$FAILED" -eq 0 ]]; then
  ok "all checks passed"
  exit 0
else
  bad "one or more checks failed"
  exit 1
fi
