#!/usr/bin/env bash
# examples/github_activity/load_day.sh — the DuckDB leg's day loader,
# declared as an external step (models/sources/raw/github_loader.yml) and
# invoked by `smelt run` once per run, ahead of every model
# (docs/specs/sources.md §"Externally-produced sources (black-box steps)").
#
# Idempotent per day via `main._loader_days`: a day already recorded is a
# no-op (exit 0). This is load-bearing, not cosmetic — a run may legitimately
# invoke this step more than once for the same day (a `--full-refresh` after
# the oracle leg has staged the day directly, or the propagated-region loop
# calling `execute_project` more than once per invocation). smelt guarantees
# no idempotence of the external program itself; this is the loader's own
# bookkeeping, exactly as a real at-least-once day loader would carry.
#
# Redelivery rule (matches scripts/bq-dogfood-loader.sh): each day D's load
# appends the real day's rows plus a deterministic 2% redelivered slice of
# day D-1's rows (`MOD(CAST(id AS BIGINT), 50) = 0`), byte-identical
# including `created_at`, stamped `ingested_date = D` in the arrival twin.
# Computed via a SQL interval rather than shell date arithmetic so a day at
# the start of the fixture range (with no D-1 rows in the sample) needs no
# special case: the redelivery clause simply matches zero rows.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SAMPLE="${SCRIPT_DIR}/seeds/github_events_sample.parquet"
DATABASE="${SCRIPT_DIR}/target/dev.duckdb"
DATE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --date)
      DATE="$2"
      shift 2
      ;;
    --database)
      DATABASE="$2"
      shift 2
      ;;
    *)
      echo "load_day.sh: unknown argument '$1'" >&2
      exit 2
      ;;
  esac
done

if [[ -z "${DATE}" ]]; then
  echo "load_day.sh: --date is required" >&2
  exit 2
fi

mkdir -p "$(dirname "${DATABASE}")"

already=$(duckdb "${DATABASE}" -json -c "
CREATE TABLE IF NOT EXISTS main._loader_days (day DATE PRIMARY KEY);
SELECT count(*) AS c FROM main._loader_days WHERE day = DATE '${DATE}';
")
count=$(echo "${already}" | grep -o '"c":[0-9]*' | grep -o '[0-9]*$')

if [[ "${count}" != "0" ]]; then
  echo "load_day.sh: day ${DATE} already loaded, skipping" >&2
  exit 0
fi

duckdb "${DATABASE}" <<SQL
BEGIN TRANSACTION;

CREATE TABLE IF NOT EXISTS main.sources_raw_github_events AS
SELECT * FROM read_parquet('${SAMPLE}') WHERE 1 = 0;

CREATE TABLE IF NOT EXISTS main.sources_raw_github_events_arrival AS
SELECT *, CAST(NULL AS DATE) AS ingested_date
FROM read_parquet('${SAMPLE}') WHERE 1 = 0;

INSERT INTO main.sources_raw_github_events
SELECT * FROM read_parquet('${SAMPLE}')
WHERE CAST(created_at AS DATE) = DATE '${DATE}'
UNION ALL
SELECT * FROM read_parquet('${SAMPLE}')
WHERE CAST(created_at AS DATE) = DATE '${DATE}' - INTERVAL 1 DAY
  AND MOD(CAST(id AS BIGINT), 50) = 0;

INSERT INTO main.sources_raw_github_events_arrival
SELECT *, DATE '${DATE}' AS ingested_date FROM read_parquet('${SAMPLE}')
WHERE CAST(created_at AS DATE) = DATE '${DATE}'
UNION ALL
SELECT *, DATE '${DATE}' AS ingested_date FROM read_parquet('${SAMPLE}')
WHERE CAST(created_at AS DATE) = DATE '${DATE}' - INTERVAL 1 DAY
  AND MOD(CAST(id AS BIGINT), 50) = 0;

INSERT INTO main._loader_days VALUES (DATE '${DATE}');

COMMIT;
SQL
