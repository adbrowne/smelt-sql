#!/usr/bin/env bash
# Cross-engine dialect audit against a live BigQuery.
#
# Unlike `bigquery-test.sh`, this does NOT let an absent credential fall through
# to a green skip: every #[test] in the BigQuery leg skips green when
# SMELT_BQ_PROJECT is absent, so a run without it would report success while
# covering nothing. This is also the point at which the sweep costs money — the
# value leg executes rather than dry-runs.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
# shellcheck disable=SC1091
source scripts/bigquery-env.sh

if [ -z "${SMELT_BQ_ACCESS_TOKEN:-}" ]; then
  echo "bigquery-dialect-audit.sh: no valid SMELT_BQ_ACCESS_TOKEN — run: bash scripts/bigquery-auth.sh" >&2
  exit 1
fi
if [ -z "${SMELT_BQ_PROJECT:-}" ]; then
  echo "bigquery-dialect-audit.sh: SMELT_BQ_PROJECT is unset — the leg would skip green and verify nothing. Run: bash scripts/bigquery-key.sh <project-id>, then source scripts/bigquery-env.sh" >&2
  exit 1
fi

# The sweep's whole output is the deliverable: every unregistered pair it names
# is a finding. `tail` would throw exactly those away — a full run reports more
# than 80 lines — so the log is written in full and only a summary is echoed.
LOG="${SMELT_BQ_AUDIT_LOG:-target/bigquery-dialect-audit.log}"
mkdir -p "$(dirname "$LOG")"

set +e
cargo test -p smelt-db --test dialect_audit --quiet -- --nocapture > "$LOG" 2>&1
status=$?
set -e

grep -E "^(COVERAGE|compared=|test result)" "$LOG" || true
echo
echo "Full sweep log: $LOG"
if [ "$status" -ne 0 ]; then
  echo
  echo "Unregistered pairs (each is a finding — give the entry an Emission verdict, or a ledger row):" >&2
  grep -E "^  [A-Z_0-9|^%/*]+ \[[A-Za-z]+\] on " "$LOG" >&2 || true
fi
exit "$status"
