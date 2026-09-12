#!/usr/bin/env bash
# refresh_sample.sh — regenerate seeds/github_events_sample.parquet from BigQuery.
#
#     bash scripts/bigquery-auth.sh            # mint a 1h token
#     bash examples/github_activity/refresh_sample.sh
#
# The committed Parquet is the DuckDB leg's input and runs in ordinary CI, so
# this script is NOT part of any test — it exists so the fixture is reproducible
# rather than mysterious. `sample.sql` beside it is the contract: the BigQuery
# loader must land exactly these rows, or the dual-target parity check compares
# two different populations.
#
# Scan cost at the pinned 30-day range: ~12 GB, about US$0.06 (the
# `payload` projection, not the day range, is what dominates it).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
out="$here/seeds/github_events_sample.parquet"
raw="$(mktemp -t github_events.XXXXXX.ndjson)"
trap 'rm -f "$raw"' EXIT

mkdir -p "$here/seeds"
bash "$repo/scripts/bq-dogfood-export.sh" "$here/sample.sql" "$raw"

# BigQuery hands back TIMESTAMP as epoch seconds; land it as a naive UTC
# TIMESTAMP so the DuckDB leg and the BigQuery leg agree on the event clock.
# The ORDER BY makes the file byte-stable across regenerations.
duckdb -c "COPY (
  SELECT id, type,
         to_timestamp(created_at) AT TIME ZONE 'UTC' AS created_at,
         actor_id, actor_login, repo_id, repo_name, org_id, public, payload
  FROM read_json_auto('${raw}')
  ORDER BY created_at, id
) TO '${out}' (FORMAT PARQUET, COMPRESSION ZSTD);"

duckdb -c "SELECT count(*) AS n_rows,
                  count(DISTINCT actor_id) AS n_actors,
                  count(DISTINCT repo_id)  AS n_repos,
                  min(created_at) AS min_ts, max(created_at) AS max_ts
           FROM '${out}';"
