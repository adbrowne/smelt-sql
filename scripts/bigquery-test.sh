#!/usr/bin/env bash
# bigquery-test.sh — run the BigQuery-gated test suites against the live warehouse.
#
#     bash scripts/bigquery-auth.sh          # mint a token (prompts for passphrase)
#     bash scripts/bigquery-test.sh [args…]  # defaults to the smoke suite
#
# Sources the BigQuery environment (token, project, PYTHONPATH for the PyO3
# adapter) and runs cargo with the `bigquery` feature. With no token minted the
# suites skip green rather than fail.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
# shellcheck disable=SC1091
source scripts/bigquery-env.sh

if [ "$#" -eq 0 ]; then
  set -- --test bigquery_smoke
fi
exec cargo test -p smelt-cli --features bigquery "$@" -- --nocapture
