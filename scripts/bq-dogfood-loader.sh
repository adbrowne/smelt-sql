#!/usr/bin/env bash
# bq-dogfood-loader.sh — derives the BigQuery dogfood loader's load SQL from
# examples/github_activity/sample.sql, rather than restating it, so the two
# can never drift apart silently
# (docs/outcomes/20260906-bigquery-dogfood-spine/phases/09-plan.md). The
# loader is external to smelt by this outcome's own "Out of scope" section —
# smelt's source declaration is the contract this script is trusted against.
#
#   bash scripts/bq-dogfood-loader.sh --emit-ddl
#   bash scripts/bq-dogfood-loader.sh --emit-sql --date <YYYY-MM-DD>
#   bash scripts/bq-dogfood-loader.sh --date <YYYY-MM-DD>       # deploy: phase 10, not yet implemented
#
# `--emit-sql`/`--emit-ddl` touch no network and need no `bq`/`gcloud` on
# PATH — they only read two files in this repo and print SQL. Phase 10
# (human-gated: creates real cloud resources) turns the emitted SQL into a
# scheduled query and wires the default execute path below; nothing in this
# script schedules or deploys anything today.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

SAMPLE_SQL="examples/github_activity/sample.sql"
README="examples/github_activity/README.md"

# Mirrors examples/github_activity/run_incremental.py's REDELIVERY_MODULUS
# (the DuckDB replay driver's own redelivery rule); asserted equal to it by
# crates/smelt-cli/tests/github_activity_loader.rs so the two legs cannot
# drift apart silently.
REDELIVERY_MODULUS=50

usage() {
  echo "usage: $0 --emit-ddl | --emit-sql --date <YYYY-MM-DD> | --date <YYYY-MM-DD>" >&2
  exit 2
}

# sample.sql's body — the first bare "SELECT" line through EOF — with the
# _TABLE_SUFFIX BETWEEN line's values replaced. Every other line is
# untouched byte-for-byte; verified by
# loader_reproduces_the_sample_projection_and_filter_verbatim.
emit_base_query() {
  local suffix_lo="$1" suffix_hi="$2"
  awk '/^SELECT$/{p=1} p' "$SAMPLE_SQL" \
    | sed -E "s/^WHERE _TABLE_SUFFIX BETWEEN '[0-9]{4}' AND '[0-9]{4}'\$/WHERE _TABLE_SUFFIX BETWEEN '${suffix_lo}' AND '${suffix_hi}'/"
}

mmdd() { date -d "$1" +%m%d; }
yyyy() { date -d "$1" +%Y; }
prev_day() { date -d "$1 - 1 day" +%Y-%m-%d; }

emit_sql() {
  local d="$1" prev suffix_lo suffix_hi base arm_filter
  prev="$(prev_day "$d")"
  if [ "$(yyyy "$d")" != "$(yyyy "$prev")" ]; then
    echo "error: --date $d is January 1st — the redelivery arm's day $prev falls in a" \
      "different year, and sample.sql's githubarchive.day.<year>* wildcard cannot span" \
      "two years" >&2
    exit 1
  fi
  suffix_lo="$(mmdd "$prev")"
  suffix_hi="$(mmdd "$d")"
  base="$(emit_base_query "$suffix_lo" "$suffix_hi")"
  arm_filter="(CAST(created_at AS DATE) = DATE '${d}' OR (CAST(created_at AS DATE) = DATE '${prev}' AND MOD(CAST(id AS BIGINT), ${REDELIVERY_MODULUS}) = 0))"

  cat <<SQL
-- Derived from ${SAMPLE_SQL} for load day ${d}: the same query, unchanged
-- except its _TABLE_SUFFIX range, scanning day ${prev}'s and day ${d}'s
-- shards. Real day-${d} rows land as-is; a deterministic 1/${REDELIVERY_MODULUS}
-- slice of day ${prev}'s rows is redelivered on purpose (the loader's declared
-- at-least-once behaviour) into both the event-time and arrival-partitioned
-- tables, stamped with today's ingested_date in the arrival twin either way.

INSERT INTO \`raw.github_events\`
SELECT * FROM (
${base}
)
WHERE ${arm_filter};

INSERT INTO \`raw.github_events_arrival\`
SELECT *, DATE '${d}' AS ingested_date FROM (
${base}
)
WHERE ${arm_filter};
SQL
}

# The retention bound is documented once in README.md ("The BigQuery
# loader" section) and parsed here — never hard-coded a second time.
parse_retention_days() {
  grep -oE 'partition_expiration_days = [0-9]+' "$README" | head -1 | grep -oE '[0-9]+'
}

emit_ddl() {
  local retention
  retention="$(parse_retention_days)"
  : "${retention:?could not parse partition_expiration_days from $README}"
  cat <<SQL
-- Day-partitioned; sets no *table* expiry (criterion 1's "no default table
-- expiration" is a dataset property this DDL never touches). Retention is
-- enforced by partition_expiration_days instead, trimming old partitions
-- without an unbounded table scan on every load.
CREATE TABLE IF NOT EXISTS \`raw.github_events\` (
  id STRING,
  type STRING,
  created_at TIMESTAMP,
  actor_id INT64,
  actor_login STRING,
  repo_id INT64,
  repo_name STRING,
  org_id INT64,
  public BOOL
)
PARTITION BY DATE(created_at)
CLUSTER BY repo_id
OPTIONS (
  partition_expiration_days = ${retention}
);

-- Arrival-partitioned twin (docs/outcomes/20260906-bigquery-dogfood-spine/
-- phases/03-plan.md): same columns plus the loader's own ingested_date stamp.
CREATE TABLE IF NOT EXISTS \`raw.github_events_arrival\` (
  id STRING,
  type STRING,
  created_at TIMESTAMP,
  actor_id INT64,
  actor_login STRING,
  repo_id INT64,
  repo_name STRING,
  org_id INT64,
  public BOOL,
  ingested_date DATE
)
PARTITION BY ingested_date
CLUSTER BY repo_id
OPTIONS (
  partition_expiration_days = ${retention}
);
SQL
}

mode=execute
d=
while [ $# -gt 0 ]; do
  case "$1" in
    --emit-ddl) mode=emit-ddl; shift ;;
    --emit-sql) mode=emit-sql; shift ;;
    --date)
      [ $# -ge 2 ] || usage
      d="$2"
      shift 2
      ;;
    -h | --help) usage ;;
    *) usage ;;
  esac
done

case "$mode" in
  emit-ddl)
    emit_ddl
    ;;
  emit-sql)
    [ -n "$d" ] || usage
    emit_sql "$d"
    ;;
  execute)
    [ -n "$d" ] || usage
    echo "error: deploying/executing the loader is phase 10 (human-gated, needs the" \
      "provisioned dogfood project) and is not implemented here — use --emit-sql/--emit-ddl" >&2
    exit 1
    ;;
esac
