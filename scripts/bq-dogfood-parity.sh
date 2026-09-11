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
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
cd "$REPO"

EXAMPLE_DIR="$REPO/examples/github_activity"
OUT_DIR="${PARITY_OUT_DIR:-$REPO/target/phase13}"
SMELT_BIN="${SMELT_BIN:-$REPO/target/debug/smelt}"
DATASET="${SMELT_BQ_DATASET:-smelt_dogfood}"

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
  bq_query - <<SQL | python3 -c 'import json,sys
for line in sys.stdin:
    print(json.loads(line)["table_name"])'
SELECT table_name
FROM \`${DATASET}.INFORMATION_SCHEMA.TABLES\`
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
  bq_query - <<SQL | python3 -c 'import json,sys
for line in sys.stdin:
    print(json.loads(line)["table_name"])'
SELECT table_name
FROM \`${DATASET}.INFORMATION_SCHEMA.TABLES\`
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
  relations) need_token; compared_relations ;;
  *)
    echo "usage: $0 {clear|duck|duck-preloaded|bq|manifest|report|relations}" >&2
    exit 2
    ;;
esac
