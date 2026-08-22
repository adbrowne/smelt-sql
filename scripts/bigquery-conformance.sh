#!/usr/bin/env bash
# bigquery-conformance.sh — run the generative maintenance-conformance
# BigQuery leg (`maintenance_conformance_bigquery`) against the live
# warehouse.
#
#     bash scripts/bigquery-auth.sh          # mint a token (prompts for passphrase)
#     bash scripts/bigquery-conformance.sh
#
# Deliberately separate from `bigquery-parity.sh`: parity is a bounded sweep
# run routinely, while this leg is slow (every statement is a network round
# trip) and rate-limited, so a session may want the whole one-hour token
# window rather than sharing it with the parity suites.
#
# Unlike `bigquery-test.sh`/`bigquery-parity.sh`, this script does NOT let an
# absent credential fall through to a green skip. A conformance sweep that
# starts without a valid token dies mid-case, having burned real quota and
# proved nothing about the cases it never reached — so this script refuses to
# start one, and names exactly what is missing and the command that fixes it.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
# shellcheck disable=SC1091
source scripts/bigquery-env.sh

if [ -z "${SMELT_BQ_ACCESS_TOKEN:-}" ]; then
  echo "bigquery-conformance.sh: no valid BigQuery access token — run: bash scripts/bigquery-auth.sh" >&2
  exit 1
fi

# SMELT_BQ_PROJECT unset is the silent-skip trap this script exists to
# prevent: every #[test] in maintenance_conformance_bigquery skips green when
# it's absent, so a run without it would report success while covering
# nothing.
if [ -z "${SMELT_BQ_PROJECT:-}" ]; then
  echo "bigquery-conformance.sh: SMELT_BQ_PROJECT is unset — the leg would skip green and verify nothing. Run: bash scripts/bigquery-key.sh <project-id>, then source scripts/bigquery-env.sh" >&2
  exit 1
fi

# Concurrency: every case's dataset is fresh and isolated
# (`BigQueryConformanceBackend::target`/`schema`/`twin_target`/`twin_schema`
# derive a fresh dataset per (family, case)), and BigQuery's per-table
# modification-quota burst limit binds PER TABLE — since no table is ever
# shared across tests, running tests concurrently does not re-trigger it.
# The token-window preflight (crates/smelt-maintenance-testkit/src/
# bigquery_session.rs) is now checked ONCE PER PROCESS against the whole
# sweep's estimated cost, not once per test, so it stays a correct bound
# regardless of how many threads run concurrently. The thread count is still
# bounded (default 4, not "however many cores this machine has") to stay
# well clear of project-level concurrent-query limits, which bind across
# every table/dataset in the project and are a different constraint than the
# per-table quota above.
SMELT_CONFORMANCE_BQ_TEST_THREADS="${SMELT_CONFORMANCE_BQ_TEST_THREADS:-4}"
exec cargo test --no-fail-fast -p smelt-cli --features bigquery \
  --test maintenance_conformance_bigquery \
  "$@" -- --test-threads="$SMELT_CONFORMANCE_BQ_TEST_THREADS" --nocapture
