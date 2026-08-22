#!/usr/bin/env bash
# bigquery-parity.sh — run every parity suite against the live BigQuery warehouse.
#
#     bash scripts/bigquery-auth.sh      # mint a token (prompts for passphrase)
#     bash scripts/bigquery-parity.sh
#
# The suites are the same ones DuckDB and Spark run; each grew a BigQuery leg
# via `TargetKind::BigQuery` in `crates/smelt-cli/tests/common/mod.rs`. With no
# token minted every BigQuery leg skips and the suites still pass on DuckDB, so
# this script is safe to run unauthenticated — it just proves less.
#
# Each suite isolates in its own dataset (`<base>_<label>_<pid>`) and drops it on
# the way out; `bigquery-env.sh` also exports a default table expiration so an
# interrupted run cannot leave tables behind indefinitely.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
# shellcheck disable=SC1091
source scripts/bigquery-env.sh

# --no-fail-fast: one failing leg must not hide the state of the others. A
# BigQuery sweep is slow and rate-limited, so a run should report the whole
# matrix rather than stopping at the first refusal.
exec cargo test --no-fail-fast -p smelt-cli --features bigquery \
  --test dual_target_harness \
  --test source_seed \
  --test seed_parity \
  --test materialization_parity \
  --test materialized_view_parity \
  --test lowering_parity \
  --test pipe_parity \
  --test merge_parity \
  --test incremental_parity \
  --test schema_evolution_parity \
  "$@" -- --nocapture
