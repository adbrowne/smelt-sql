#!/usr/bin/env bash
# bq-dogfood-parity.sh — drive `examples/github_activity` over the fixture's
# thirty windows on BOTH targets and snapshot the compared relations, so the
# offline comparator in `crates/smelt-cli/tests/github_activity_dual_target.rs`
# can prove criterion 6 ("the two targets agree") over one shared population.
#
#     source scripts/bq-dogfood-env.sh
#     export SMELT_BQ_ACCESS_TOKEN=$(gcloud auth application-default print-access-token \
#       --impersonate-service-account=smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com)
#     bash scripts/bq-dogfood-parity.sh clear
#     bash scripts/bq-dogfood-parity.sh duck
#     bash scripts/bq-dogfood-parity.sh bq
#     bash scripts/bq-dogfood-parity.sh manifest
#
# The `bq` stage runs for longer than an impersonated token lives, so set
# PARITY_TOKEN_CMD to a command that prints a fresh one and it is re-minted
# before every window and every snapshot:
#
#     export PARITY_TOKEN_CMD='gcloud auth application-default print-access-token \
#       --impersonate-service-account=smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com'
#
# Blast radius, enforced by construction:
#   * `clear` drops ONLY the names it prints, and refuses outright if that list
#     ever contains `github_events`, `github_events_arrival` or anything outside
#     the `smelt_dogfood` dataset. Phase 17 owns the source; this script reads it.
#   * Nothing here touches `githubarchive`.
#   * Both legs run with the same two `-e` exclusions, so the relation sets are
#     equal by construction rather than by tolerance.
#
# The DuckDB leg is the committed replay (`examples/github_activity/run_incremental.py`),
# not a re-implementation of it. The BigQuery leg is the same thirty windows of
# the same `smelt run`, with `--target bigquery`.
#
# The same driver also carries the equivalence-invariant leg, which reuses the
# incremental snapshots `bq` already exported rather than re-running them:
#
#     bash scripts/bq-dogfood-oracle-dataset.sh create   # human credential
#     bash scripts/bq-dogfood-parity.sh oracle-precheck
#     bash scripts/bq-dogfood-parity.sh oracle
#     bash scripts/bq-dogfood-parity.sh oracle-manifest
#     bash scripts/bq-dogfood-oracle-dataset.sh drop     # scaffolding, not history
#
# See the section comment above `stage_oracle_precheck` for what that leg claims
# and why the oracle needs its own dataset but the SAME source tables.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
cd "$REPO"

EXAMPLE_DIR="$REPO/examples/github_activity"
OUT_DIR="${PARITY_OUT_DIR:-$REPO/target/phase13}"
SMELT_BIN="${SMELT_BIN:-$REPO/target/debug/smelt}"
DATASET="${SMELT_BQ_DATASET:-smelt_dogfood}"
# The full-refresh oracle's own dataset (`oracle` stage). Declared in the
# committed project as the `bigquery_oracle` target, which reads the SAME
# physical source tables out of $DATASET and writes only its outputs here.
ORACLE_DATASET="${PARITY_ORACLE_DATASET:-smelt_dogfood_oracle}"

START_DATE="${PARITY_START_DATE:-2026-08-05}"
DAYS="${PARITY_DAYS:-30}"
# Declared comparison points (D2). Measured, not assumed — see
# docs/outcomes/20260906-bigquery-dogfood-spine/phases/13-summary.md.
CHECKPOINTS="${PARITY_CHECKPOINTS:-1,2,3,5,10,20,30}"

# The pair GoogleSQL refuses at compile time on the INTERVAL RANGE lookback
# frame, excluded on BOTH legs. Kept in lockstep with EXCLUDED_MODELS in
# crates/smelt-cli/tests/github_activity_dual_target.rs.
EXCLUDE_MODELS=(silver.actor_sessions marts.daily_active_contributors)

mkdir -p "$OUT_DIR"

need_token() {
  : "${SMELT_BQ_ACCESS_TOKEN:?no token — see the header of this script}"
  : "${SMELT_BQ_PROJECT:?no project — run: source scripts/bq-dogfood-env.sh}"
}

# An impersonated access token lives an hour; the thirty-window BigQuery leg
# takes longer than that. Set PARITY_TOKEN_CMD to a command that prints a fresh
# token and it is re-evaluated before every window, so the schedule cannot die
# on expiry two thirds of the way through. Unset, the token is whatever the
# caller exported (fine for a short run, and for every other stage).
refresh_token() {
  [[ -n "${PARITY_TOKEN_CMD:-}" ]] || return 0
  local tok
  tok="$(eval "$PARITY_TOKEN_CMD")" || {
    echo "PARITY_TOKEN_CMD failed" >&2
    return 1
  }
  [[ -n "$tok" ]] || {
    echo "PARITY_TOKEN_CMD printed nothing" >&2
    return 1
  }
  export SMELT_BQ_ACCESS_TOKEN="$tok"
}

# One GoogleSQL statement; rows as JSON lines on stdout.
bq_query() {
  python3 "$REPO/scripts/bq_dogfood_query.py" "$@"
}

# The relations the sweep compares, discovered generically from BigQuery's own
# catalogue with the same exclusion rules the comparator applies — never a
# hardcoded model list.
compared_relations() {
  local ds="${1:-$DATASET}"
  bq_query - <<SQL | python3 -c 'import json,sys
for line in sys.stdin:
    print(json.loads(line)["table_name"])'
SELECT table_name
FROM \`${ds}.INFORMATION_SCHEMA.TABLES\`
WHERE table_type = 'BASE TABLE'
  AND NOT STARTS_WITH(table_name, 'sources_')
  AND NOT STARTS_WITH(table_name, '_smelt_')
  AND NOT ENDS_WITH(table_name, '__tombstones')
  AND table_name NOT IN ('github_events', 'github_events_arrival', '_loader_days')
ORDER BY table_name
SQL
}

# Everything `clear` is allowed to drop: the model tables, their tombstone
# siblings, and the engine-resident bookkeeping. Discovered, then screened.
droppable_relations() {
  local ds="${1:-$DATASET}"
  bq_query - <<SQL | python3 -c 'import json,sys
for line in sys.stdin:
    print(json.loads(line)["table_name"])'
SELECT table_name
FROM \`${ds}.INFORMATION_SCHEMA.TABLES\`
WHERE table_name NOT IN ('github_events', 'github_events_arrival')
ORDER BY table_name
SQL
}

stage_clear() {
  need_token
  local names=()
  mapfile -t names < <(droppable_relations)
  echo "about to drop ${#names[@]} table(s) from ${SMELT_BQ_PROJECT}.${DATASET}:"
  printf '  %s\n' "${names[@]}"
  for n in "${names[@]}"; do
    case "$n" in
      github_events|github_events_arrival|*.*|"")
        echo "REFUSING: '$n' is not droppable by this script" >&2
        exit 1
        ;;
    esac
  done
  for n in "${names[@]}"; do
    echo "DROP TABLE \`${DATASET}.${n}\`" | bq_query - >/dev/null
    echo "dropped ${n}"
  done
  rm -rf "$EXAMPLE_DIR/.smelt/targets/bigquery"
  echo "removed $EXAMPLE_DIR/.smelt/targets/bigquery"
}

stage_duck() {
  local exclude_args=()
  for m in "${EXCLUDE_MODELS[@]}"; do exclude_args+=(--exclude "$m"); done
  PATH="$(dirname "$SMELT_BIN"):$PATH" python3 "$EXAMPLE_DIR/run_incremental.py" \
    --start-date "$START_DATE" --days "$DAYS" \
    --snapshot-dir "$OUT_DIR/duck" --snapshot-after "$CHECKPOINTS" \
    --skip-tests "${exclude_args[@]}" \
    --report "$OUT_DIR/duck_replay.json"
}

# The attribution leg (D2). Arrival order differs between the two targets —
# BigQuery's source is fully populated before the first window, DuckDB's grows a
# day at a time — and that is a variable to control, not to ignore. This runs the
# same thirty windows on DuckDB with the source staged up front, so a difference
# that survives here is about the engine and one that vanishes is about arrival
# order. `load_day.sh` is unchanged: its own per-day idempotence turns the
# declared external step into a no-op once the day is already recorded.
stage_duck_preloaded() {
  local exclude_args=()
  for m in "${EXCLUDE_MODELS[@]}"; do exclude_args+=(--exclude "$m"); done
  PATH="$(dirname "$SMELT_BIN"):$PATH" python3 "$EXAMPLE_DIR/run_incremental.py" \
    --start-date "$START_DATE" --days "$DAYS" --preload-source \
    --snapshot-dir "$OUT_DIR/duck-preloaded" --snapshot-after "$CHECKPOINTS" \
    --skip-tests "${exclude_args[@]}" \
    --report "$OUT_DIR/duck_preloaded_replay.json"
}

stage_bq() {
  need_token
  local exclude_args=()
  for m in "${EXCLUDE_MODELS[@]}"; do exclude_args+=(-e "$m"); done
  # Window numbers stay absolute across a resume, so a checkpoint keeps its
  # label and the report's schedule column means the same thing either way.
  local first="${PARITY_RESUME_FROM:-1}"
  local n day nextday snap_dir rel
  for ((n = first; n <= DAYS; n++)); do
    day="$(date -u -d "$START_DATE + $((n - 1)) day" +%F)"
    nextday="$(date -u -d "$START_DATE + $n day" +%F)"
    echo "=== window $n/$DAYS  [$day .. $nextday) $(date -u +%T) ==="
    refresh_token
    ( cd "$EXAMPLE_DIR" && "$SMELT_BIN" run --target bigquery \
        --event-time-start "$day" --event-time-end "$nextday" "${exclude_args[@]}" )
    if [[ ",$CHECKPOINTS," == *",$n,"* ]]; then
      snap_dir="$OUT_DIR/bq/$(printf 'w%02d' "$n")"
      mkdir -p "$snap_dir"
      refresh_token
      while read -r rel; do
        [[ -z "$rel" ]] && continue
        printf 'SELECT * FROM `%s.%s`\n' "$DATASET" "$rel" > "$snap_dir/$rel.sql"
        python3 "$REPO/scripts/bq_dogfood_export.py" \
          "$snap_dir/$rel.sql" "$snap_dir/$rel.ndjson"
      done < <(compared_relations)
      echo "snapshot -> $snap_dir"
    fi
  done
}

# ---------------------------------------------------------------------------
# The equivalence-invariant legs (phase 14)
# ---------------------------------------------------------------------------
# The dual-target sweep above asks "do two engines agree with each other". The
# stages below ask the other question: does ONE engine's incrementally-
# maintained state equal its own full refresh over the inputs seen so far
# (`docs/specs/incremental_models.md` §"The equivalence invariant")?
#
# The incremental side is already on disk — `stage_bq` exported it at every
# declared checkpoint. Only the oracle side is new: at checkpoint k, refresh
# every model from scratch over [START_DATE, day k+1) on the `bigquery_oracle`
# target and export the result. Both sides then land into DuckDB, typed from
# the same reference database, and are differenced by the same comparator.
#
# The oracle target reads the SAME physical source tables (the
# `bigquery_oracle:` entries in the two source YAMLs). Without them the source
# would resolve to a table nothing creates and every oracle would be empty —
# a vacuous pass, which is the failure mode this leg exists to rule out.

# Precondition for the oracle leg: the dataset exists and this credential can
# read its catalogue.
#
# Creating it is NOT this script's job and cannot be — the dogfood service
# account was provisioned with `roles/bigquery.jobUser` plus WRITER on one
# dataset and no `bigquery.datasets.create` at all ("never creates datasets of
# its own", scripts/bq-dogfood-provision.sh). Dataset lifecycle runs under the
# human credential in `scripts/bq-dogfood-oracle-dataset.sh {create,drop}`;
# this stage only refuses to start a sweep that would otherwise fail
# mid-flight.
stage_oracle_precheck() {
  need_token
  local n
  n="$(droppable_relations "$ORACLE_DATASET" | wc -l)" || {
    echo "the oracle dataset ${ORACLE_DATASET} is not readable — create it with:" >&2
    echo "  bash scripts/bq-dogfood-oracle-dataset.sh create" >&2
    exit 1
  }
  echo "oracle dataset ${SMELT_BQ_PROJECT}.${ORACLE_DATASET} is present (${n} table(s))"
}

# Empty the oracle dataset. Each checkpoint's oracle must be a refresh from
# nothing, exactly as the DuckDB oracle stages a fresh workspace per window
# (`github_activity_oracle.rs`), so nothing carries between checkpoints —
# neither a table nor the interval ledger.
#
# Screened like `clear`: this refuses if the discovered list ever names a
# source table or anything qualified. It can only ever see the oracle dataset,
# which holds no sources at all, but the screen is cheap and the blast radius
# it guards is the whole point.
oracle_clear() {
  local names=() n
  mapfile -t names < <(droppable_relations "$ORACLE_DATASET")
  for n in "${names[@]}"; do
    case "$n" in
      github_events|github_events_arrival|*.*|"")
        echo "REFUSING: '$n' is not droppable by this script" >&2
        exit 1
        ;;
    esac
    echo "DROP TABLE \`${ORACLE_DATASET}.${n}\`" | bq_query - >/dev/null
  done
  rm -rf "$EXAMPLE_DIR/.smelt/targets/bigquery_oracle"
  echo "oracle dataset emptied (${#names[@]} table(s))"
}

stage_oracle() {
  need_token
  local oracle_out="${PARITY_ORACLE_OUT_DIR:-$OUT_DIR}"
  local exclude_args=()
  for m in "${EXCLUDE_MODELS[@]}"; do exclude_args+=(-e "$m"); done
  local n nextday snap_dir rel
  for n in ${CHECKPOINTS//,/ }; do
    nextday="$(date -u -d "$START_DATE + $n day" +%F)"
    echo "=== oracle checkpoint $n  [$START_DATE .. $nextday) full refresh $(date -u +%T) ==="
    refresh_token
    oracle_clear
    refresh_token
    ( cd "$EXAMPLE_DIR" && "$SMELT_BIN" run --target bigquery_oracle --full-refresh \
        --event-time-start "$START_DATE" --event-time-end "$nextday" "${exclude_args[@]}" )
    snap_dir="$oracle_out/oracle/$(printf 'w%02d' "$n")"
    mkdir -p "$snap_dir"
    refresh_token
    while read -r rel; do
      [[ -z "$rel" ]] && continue
      printf 'SELECT * FROM `%s.%s`\n' "$ORACLE_DATASET" "$rel" > "$snap_dir/$rel.sql"
      python3 "$REPO/scripts/bq_dogfood_export.py" \
        "$snap_dir/$rel.sql" "$snap_dir/$rel.ndjson"
    done < <(compared_relations "$ORACLE_DATASET")
    echo "oracle snapshot -> $snap_dir"
  done
}

# The equivalence sweep's manifest: per checkpoint, the incremental snapshot
# `stage_bq` exported, the oracle snapshot `stage_oracle` exported, and the
# DuckDB database that supplies the declared types BOTH are landed under.
stage_oracle_manifest() {
  CPS="$CHECKPOINTS" OUT="$OUT_DIR" ORACLE_OUT="${PARITY_ORACLE_OUT_DIR:-$OUT_DIR}" \
  START="$START_DATE" MANIFEST="${PARITY_ORACLE_MANIFEST:-$REPO/target/phase14/equivalence-manifest.json}" \
  python3 - <<'PY'
import json, os, pathlib
from datetime import date, timedelta
out = pathlib.Path(os.environ["OUT"])
oracle_out = pathlib.Path(os.environ["ORACLE_OUT"])
start = date.fromisoformat(os.environ["START"])
entries = []
for n in (int(x) for x in os.environ["CPS"].split(",") if x.strip()):
    day = start + timedelta(days=n - 1)
    entries.append({
        "label": f"w{n:02d}",
        "window": n,
        "day": day.isoformat(),
        # The type reference, not a comparison side: both BigQuery snapshots
        # are landed under these declared types so they are byte-comparable.
        "types_db_path": str(out / "duck" / f"w{n:02d}.duckdb"),
        "incr_ndjson_dir": str(out / "bq" / f"w{n:02d}"),
        "oracle_ndjson_dir": str(oracle_out / "oracle" / f"w{n:02d}"),
    })
path = pathlib.Path(os.environ["MANIFEST"])
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps({"checkpoints": entries}, indent=2) + "\n")
print(f"wrote {path} with {len(entries)} checkpoint(s)")
PY
}

stage_manifest() {
  CPS="$CHECKPOINTS" OUT="$OUT_DIR" START="$START_DATE" \
  DUCK_DIR="${PARITY_DUCK_DIR:-duck}" MANIFEST="${PARITY_MANIFEST_NAME:-parity-manifest.json}" \
  python3 - <<'PY'
import json, os, pathlib
out = pathlib.Path(os.environ["OUT"])
from datetime import date, timedelta
start = date.fromisoformat(os.environ["START"])
duck_dir = os.environ["DUCK_DIR"]
entries = []
for n in (int(x) for x in os.environ["CPS"].split(",") if x.strip()):
    day = start + timedelta(days=n - 1)
    entries.append({
        "label": f"w{n:02d}",
        "window": n,
        "day": day.isoformat(),
        "duck_db_path": str(out / duck_dir / f"w{n:02d}.duckdb"),
        "ndjson_dir": str(out / "bq" / f"w{n:02d}"),
    })
path = out / os.environ["MANIFEST"]
path.write_text(json.dumps({"checkpoints": entries}, indent=2) + "\n")
print(f"wrote {path} with {len(entries)} checkpoint(s)")
PY
}

# Render the markdown twin of the JSON report the live sweep
# (`github_activity_dual_target::duckdb_and_bigquery_agree_on_every_model`)
# writes. One row per relation, one column per compared checkpoint; a cell is
# `=` when the multiset difference is zero in both directions and
# `-<duck_only>/+<bq_only>` when it is not.
stage_report() {
  REPO="$REPO" python3 - <<'PY'
import json, os, pathlib
repo = pathlib.Path(os.environ["REPO"])
base = repo / "docs/outcomes/20260906-bigquery-dogfood-spine/phases"
report = json.loads((base / "13-parity.json").read_text())
cps = report["checkpoints"]
rels = [r["relation"] for r in cps[0]["relations"]]
lines = ["| relation | " + " | ".join(f'{c["label"]} ({c["day"]})' for c in cps) + " |",
         "|---" * (len(cps) + 1) + "|"]
for rel in rels:
    cells = []
    for c in cps:
        d = next(r for r in c["relations"] if r["relation"] == rel)
        cells.append("=" if d["duck_only"] == 0 and d["bq_only"] == 0
                     else f'-{d["duck_only"]}/+{d["bq_only"]}')
    lines.append(f"| `{rel}` | " + " | ".join(cells) + " |")
lines.append("")
lines.append("| relation | " + " | ".join(c["label"] for c in cps) + " |")
lines.append("|---" * (len(cps) + 1) + "|")
for rel in rels:
    row = []
    for c in cps:
        d = next(r for r in c["relations"] if r["relation"] == rel)
        row.append(str(d["duck_rows"]) if d["duck_rows"] == d["bq_rows"]
                   else f'{d["duck_rows"]}/{d["bq_rows"]}')
    lines.append(f"| `{rel}` | " + " | ".join(row) + " |")
out = base / "13-parity.md"
out.write_text("\n".join(lines) + "\n")
print(f"wrote {out}")
PY
}

case "${1:-}" in
  clear) stage_clear ;;
  report) stage_report ;;
  duck) stage_duck ;;
  duck-preloaded) stage_duck_preloaded ;;
  bq) stage_bq ;;
  manifest) stage_manifest ;;
  oracle-precheck) stage_oracle_precheck ;;
  oracle) stage_oracle ;;
  oracle-manifest) stage_oracle_manifest ;;
  relations) need_token; compared_relations "${2:-$DATASET}" ;;
  *)
    echo "usage: $0 {clear|duck|duck-preloaded|bq|manifest|report|relations|\
oracle-precheck|oracle|oracle-manifest}" >&2
    exit 2
    ;;
esac
