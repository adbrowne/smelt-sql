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

cargo test -p smelt-db --test dialect_audit --quiet -- --nocapture 2>&1 | tail -80
