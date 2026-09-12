#!/usr/bin/env bash
# dbx-dogfood-parity.sh — drive `examples/github_activity` on DuckDB and
# snapshot the Databricks state phases 5-7b already built over the same
# fixture days, so the offline comparator in
# `crates/smelt-cli/tests/github_activity_dual_target.rs` can prove
# criterion 7 ("the two targets agree") over one shared population.
# Modelled on scripts/bq-dogfood-parity.sh, generalised over the target per
# `crates/smelt-cli/tests/parity_support/mod.rs`.
#
#     source scripts/dbx-dogfood-env.sh
#     bash scripts/dbx-auth.sh   # mint/refresh the one-hour OAuth token
#     bash scripts/dbx-dogfood-parity.sh duck
#     bash scripts/dbx-dogfood-parity.sh dbx-snapshot
#     bash scripts/dbx-dogfood-parity.sh manifest
#     SMELT_DBX_DOGFOOD_LIVE=1 DBX_PARITY_MANIFEST="$PWD/target/phase8/parity-manifest.json" \
#       cargo test -p smelt-cli --test github_activity_dual_target \
#       duckdb_and_databricks_agree_on_every_model -- --nocapture
#     bash scripts/dbx-dogfood-parity.sh report
#
# Unlike the BigQuery driver, there is **no destructive stage at all**: the
# Databricks state phases 5-7b built (a full refresh plus three incremental
# windows, landing eight fixture days) is the thing under test, and this
# script never drops or re-creates it. `dbx-snapshot` only ever issues
# `SELECT`s (scripts/dbx_dogfood_export.py).
#
# The DuckDB leg replays the SAME eight fixture days as a fresh full-refresh
# run — `examples/github_activity/run_incremental.py`, not a
# re-implementation of it — so the comparison is over identical inputs. The
# Databricks leg is whatever `smelt run --target databricks` already left in
# `workspace.smelt_dogfood`.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
cd "$REPO"

EXAMPLE_DIR="$REPO/examples/github_activity"
OUT_DIR="${PARITY_OUT_DIR:-$REPO/target/phase8}"
SMELT_BIN="${SMELT_BIN:-$REPO/target/debug/smelt}"

START_DATE="${PARITY_START_DATE:-2026-08-05}"
DAYS="${PARITY_DAYS:-8}"
# The single comparison point: after all eight fixture days. Databricks'
# state (phases 5-7b) is likewise the end state of three incremental windows
# over the same eight days, not a per-window replay — there is nothing here
# resembling the BigQuery driver's thirty-checkpoint schedule.
CHECKPOINTS="${PARITY_CHECKPOINTS:-8}"

# Phases 6b-6f closed every construct Spark/Databricks refused, so nothing is
# excluded here — kept in lockstep with DATABRICKS_EXCLUDED_MODELS in
# crates/smelt-cli/tests/github_activity_dual_target.rs
# (databricks_sweep_compares_every_model asserts this literal array is empty).
EXCLUDE_MODELS=()

mkdir -p "$OUT_DIR"

stage_duck() {
  local exclude_args=()
  for m in "${EXCLUDE_MODELS[@]:-}"; do [[ -n "$m" ]] && exclude_args+=(--exclude "$m"); done
  PATH="$(dirname "$SMELT_BIN"):$PATH" python3 "$EXAMPLE_DIR/run_incremental.py" \
    --start-date "$START_DATE" --days "$DAYS" --window-days 1 \
    --first-full-refresh \
    --snapshot-dir "$OUT_DIR/duck" --snapshot-after "$CHECKPOINTS" \
    "${exclude_args[@]}" \
    --report "$OUT_DIR/duck_replay.json"
}

# Read-only: exports the Databricks state that is ALREADY there (phases 5-7b
# built it) into typed NDJSON. Never drops or writes anything on the
# workspace side.
stage_dbx_snapshot() {
  local snap_dir="$OUT_DIR/dbx"
  mkdir -p "$snap_dir"
  # shellcheck source=scripts/dbx-dogfood-env.sh
  . "$REPO/scripts/dbx-dogfood-env.sh" >&2
  local python_bin="$REPO/.smelt-dbx-venv/bin/python"
  [[ -x "$python_bin" ]] || python_bin="python3"
  "$python_bin" "$REPO/scripts/dbx_dogfood_export.py" "$snap_dir"
}

# The manifest the live test (duckdb_and_databricks_agree_on_every_model)
# reads: the single checkpoint's DuckDB snapshot and the Databricks export
# directory. Same shape as bq-dogfood-parity.sh's manifest stage.
stage_manifest() {
  CPS="$CHECKPOINTS" OUT="$OUT_DIR" START="$START_DATE" \
  MANIFEST="${PARITY_MANIFEST_NAME:-parity-manifest.json}" \
  python3 - <<'PY'
import json, os, pathlib
from datetime import date, timedelta
out = pathlib.Path(os.environ["OUT"])
start = date.fromisoformat(os.environ["START"])
entries = []
for n in (int(x) for x in os.environ["CPS"].split(",") if x.strip()):
    day = start + timedelta(days=n - 1)
    entries.append({
        "label": f"w{n:02d}",
        "window": n,
        "day": day.isoformat(),
        "duck_db_path": str(out / "duck" / f"w{n:02d}.duckdb"),
        "ndjson_dir": str(out / "dbx"),
    })
path = out / os.environ["MANIFEST"]
path.write_text(json.dumps({"checkpoints": entries}, indent=2) + "\n")
print(f"wrote {path} with {len(entries)} checkpoint(s)")
PY
}

# Render the markdown twin of the JSON report the live sweep
# (`github_activity_dual_target::duckdb_and_databricks_agree_on_every_model`)
# writes. One row per relation; a cell is `=` when the multiset difference is
# zero in both directions and `-<duck_only>/+<dbx_only>` when it is not.
stage_report() {
  REPO="$REPO" REPORT_JSON="${PARITY_REPORT_JSON:-08-parity.json}" \
  REPORT_MD="${PARITY_REPORT_MD:-08-parity.md}" python3 - <<'PY'
import json, os, pathlib
repo = pathlib.Path(os.environ["REPO"])
base = repo / "docs/outcomes/20260912-databricks-dogfood-spine/phases"
report = json.loads((base / os.environ["REPORT_JSON"]).read_text())
cps = report["checkpoints"]
rels = [r["relation"] for r in cps[0]["relations"]]
lines = ["| relation | " + " | ".join(f'{c["label"]} ({c["day"]})' for c in cps) + " |",
         "|---" * (len(cps) + 1) + "|"]
for rel in rels:
    cells = []
    for c in cps:
        d = next(r for r in c["relations"] if r["relation"] == rel)
        cells.append("=" if d["duck_only"] == 0 and d["dbx_only"] == 0
                     else f'-{d["duck_only"]}/+{d["dbx_only"]}')
    lines.append(f"| `{rel}` | " + " | ".join(cells) + " |")
lines.append("")
lines.append("| relation | " + " | ".join(c["label"] for c in cps) + " |")
lines.append("|---" * (len(cps) + 1) + "|")
for rel in rels:
    row = []
    for c in cps:
        d = next(r for r in c["relations"] if r["relation"] == rel)
        row.append(str(d["duck_rows"]) if d["duck_rows"] == d["dbx_rows"]
                   else f'{d["duck_rows"]}/{d["dbx_rows"]}')
    lines.append(f"| `{rel}` | " + " | ".join(row) + " |")
out = base / os.environ["REPORT_MD"]
out.write_text("\n".join(lines) + "\n")
print(f"wrote {out}")
PY
}

case "${1:-}" in
  duck) stage_duck ;;
  dbx-snapshot) stage_dbx_snapshot ;;
  manifest) stage_manifest ;;
  report) stage_report ;;
  *)
    echo "usage: $0 {duck|dbx-snapshot|manifest|report}" >&2
    exit 2
    ;;
esac
