#!/usr/bin/env bash
# dbx-dogfood-oracle.sh — the equivalence-invariant leg on Databricks
# (docs/outcomes/20260912-databricks-dogfood-spine/outcome.md criterion 8):
# does each model's incrementally-maintained state equal a full refresh over
# the inputs seen so far? Modelled on the oracle stages of
# scripts/bq-dogfood-parity.sh, generalised over the target per
# crates/smelt-cli/tests/parity_support/mod.rs.
#
# The loader lands one fixture day at a time, so the source holds exactly the
# inputs seen so far at every checkpoint (docs/outcomes/
# 20260912-databricks-dogfood-spine/phases/09a-plan.md, decision log entry
# "Oracle validity is free on this target"). Three consecutive windows land
# three new fixture days (2026-08-13/14/15, windows 9-11) on top of the eight
# phases 5-7b already loaded, and a fresh `--full-refresh` on the
# `databricks_oracle` target is compared against the `databricks` target's
# incrementally-maintained state at each one.
#
#     source scripts/dbx-dogfood-env.sh
#     bash scripts/dbx-auth.sh   # mint/refresh the one-hour OAuth token
#     bash scripts/dbx-dogfood-oracle.sh duck-types
#     bash scripts/dbx-dogfood-oracle.sh window 9
#     bash scripts/dbx-dogfood-oracle.sh oracle 9
#     bash scripts/dbx-dogfood-oracle.sh snapshot 9
#     # ...repeat window/oracle/snapshot for 10 and 11...
#     bash scripts/dbx-dogfood-oracle.sh manifest
#     SMELT_DBX_DOGFOOD_LIVE=1 EQUIVALENCE_MANIFEST="$PWD/target/phase9/equivalence-manifest.json" \
#       EQUIVALENCE_REPORT_OUT="$PWD/docs/outcomes/20260912-databricks-dogfood-spine/phases/09b-equivalence.json" \
#       cargo test -p smelt-cli --test github_activity_dbx_oracle \
#       databricks_incremental_matches_its_oracle_at_every_window -- --nocapture
#     bash scripts/dbx-dogfood-oracle.sh report
#
# `window` and `oracle` are the only stages that write to the workspace, and
# each only ever runs `smelt run` — never a hand-issued DDL/DML statement.
# `snapshot` is read-only (delegates to scripts/dbx_dogfood_export.py, which
# issues only SELECTs).
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
cd "$REPO"

EXAMPLE_DIR="$REPO/examples/github_activity"
OUT_DIR="${ORACLE_OUT_DIR:-$REPO/target/phase9}"
SMELT_BIN="${SMELT_BIN:-$REPO/target/debug/smelt}"

# Kept in lockstep with crates/smelt-cli/tests/github_activity_dbx_oracle.rs's
# the_oracle_driver_declares_the_checkpoint_schedule_the_sweep_expects.
START_DATE="${ORACLE_START_DATE:-2026-08-05}"
# Windows 9-11: the three fixture days phases 5-7b had not yet landed.
CHECKPOINTS="${ORACLE_CHECKPOINTS:-9,10,11}"

window_day() { date -u -d "$START_DATE + $(($1 - 1)) day" +%F; }
window_end() { date -u -d "$START_DATE + $1 day" +%F; }

venv_python() {
  local python_bin="$REPO/.smelt-dbx-venv/bin/python"
  [[ -x "$python_bin" ]] || python_bin="python3"
  echo "$python_bin"
}

# Delegates to the parity script's own `duck` stage for the type-reference
# databases at windows 9-11 — the same DuckDB replay, not a re-implementation.
stage_duck_types() {
  PARITY_START_DATE="$START_DATE" PARITY_DAYS=11 PARITY_CHECKPOINTS="$CHECKPOINTS" \
    PARITY_OUT_DIR="$OUT_DIR" \
    bash "$REPO/scripts/dbx-dogfood-parity.sh" duck
}

# Lands fixture day `n` through the external loader, then runs that window on
# the `databricks` target — the incremental leg.
stage_window() {
  local n="${1:?usage: $0 window <n>}" day end python_bin
  day="$(window_day "$n")"
  end="$(window_end "$n")"
  # shellcheck source=scripts/dbx-dogfood-env.sh
  . "$REPO/scripts/dbx-dogfood-env.sh" >&2
  python_bin="$(venv_python)"
  "$python_bin" "$REPO/scripts/dbx-dogfood-loader.py" --date "$day"
  ( cd "$EXAMPLE_DIR" && "$SMELT_BIN" run --target databricks \
      --event-time-start "$day" --event-time-end "$end" )
}

# A full refresh over [START_DATE, window n's end) on the `databricks_oracle`
# target — the oracle leg. The source at this point holds exactly the days
# `stage_window` has landed through window `n`, which is what makes this a
# valid oracle for "the inputs seen so far" (see the module doc of
# github_activity_dbx_oracle.rs).
stage_oracle() {
  local n="${1:?usage: $0 oracle <n>}" end
  end="$(window_end "$n")"
  # shellcheck source=scripts/dbx-dogfood-env.sh
  . "$REPO/scripts/dbx-dogfood-env.sh" >&2
  # --allow-full-refresh: each checkpoint re-runs a --full-refresh over the
  # SAME smelt_dogfood_oracle tables the previous checkpoint already built
  # (unlike the BigQuery oracle, which drops and recreates a scratch
  # dataset), so from the second checkpoint on stored output already exists
  # and the retention gate (docs/specs/sources.md §Semantics 5) requires an
  # explicit operator license for a whole-table recompute. Harmless here: at
  # 11 days deep the run window is far inside the sources' 45-day retention,
  # so the license's reported loss is a formality, not an actual gap.
  ( cd "$EXAMPLE_DIR" && "$SMELT_BIN" run --target databricks_oracle --full-refresh \
      --allow-full-refresh \
      --event-time-start "$START_DATE" --event-time-end "$end" )
}

# Read-only: exports both schemas as they stand after `window`/`oracle` ran.
stage_snapshot() {
  local n="${1:?usage: $0 snapshot <n>}" label incr_dir oracle_dir python_bin
  label="$(printf 'w%02d' "$n")"
  incr_dir="$OUT_DIR/incr/$label"
  oracle_dir="$OUT_DIR/oracle/$label"
  mkdir -p "$incr_dir" "$oracle_dir"
  # shellcheck source=scripts/dbx-dogfood-env.sh
  . "$REPO/scripts/dbx-dogfood-env.sh" >&2
  python_bin="$(venv_python)"
  SMELT_DBX_SCHEMA=smelt_dogfood "$python_bin" "$REPO/scripts/dbx_dogfood_export.py" "$incr_dir"
  SMELT_DBX_SCHEMA=smelt_dogfood_oracle "$python_bin" "$REPO/scripts/dbx_dogfood_export.py" "$oracle_dir"
}

# The equivalence sweep's manifest: per checkpoint, the incremental snapshot
# and the oracle snapshot `stage_snapshot` exported, the DuckDB database that
# supplies the declared types both are landed under, and the source's own day
# count — the measurement `assert_source_covers_window` checks against the
# window number.
stage_manifest() {
  CPS="$CHECKPOINTS" OUT="$OUT_DIR" START="$START_DATE" \
  MANIFEST="${ORACLE_MANIFEST:-$OUT_DIR/equivalence-manifest.json}" \
  python3 - <<'PY'
import json, os, pathlib
from datetime import date, timedelta
out = pathlib.Path(os.environ["OUT"])
start = date.fromisoformat(os.environ["START"])
entries = []
for n in (int(x) for x in os.environ["CPS"].split(",") if x.strip()):
    day = start + timedelta(days=n - 1)
    label = f"w{n:02d}"
    entries.append({
        "label": label,
        "window": n,
        "day": day.isoformat(),
        "types_db_path": str(out / "duck" / f"{label}.duckdb"),
        "incr_ndjson_dir": str(out / "incr" / label),
        "oracle_ndjson_dir": str(out / "oracle" / label),
        # The loader lands one day at a time starting at START, so by window
        # n the source holds exactly n days — measured here, not assumed.
        "source_days_loaded": n,
    })
path = pathlib.Path(os.environ["MANIFEST"])
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps({"checkpoints": entries}, indent=2) + "\n")
print(f"wrote {path} with {len(entries)} checkpoint(s)")
PY
}

# Render the markdown twin of the JSON report the live sweep
# (github_activity_dbx_oracle::databricks_incremental_matches_its_oracle_at_every_window)
# writes. One row per relation; a cell is `=` when the multiset difference is
# zero in both directions and `-<incr_only>/+<oracle_only>` when it is not.
stage_report() {
  REPO="$REPO" REPORT_JSON="${ORACLE_REPORT_JSON:-09b-equivalence.json}" \
  REPORT_MD="${ORACLE_REPORT_MD:-09b-equivalence.md}" python3 - <<'PY'
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
        cells.append("=" if d["incr_only"] == 0 and d["oracle_only"] == 0
                     else f'-{d["incr_only"]}/+{d["oracle_only"]}')
    lines.append(f"| `{rel}` | " + " | ".join(cells) + " |")
out = base / os.environ["REPORT_MD"]
out.write_text("\n".join(lines) + "\n")
print(f"wrote {out}")
PY
}

case "${1:-}" in
  duck-types) stage_duck_types ;;
  window) stage_window "${2:-}" ;;
  oracle) stage_oracle "${2:-}" ;;
  snapshot) stage_snapshot "${2:-}" ;;
  manifest) stage_manifest ;;
  report) stage_report ;;
  *)
    echo "usage: $0 {duck-types|window <n>|oracle <n>|snapshot <n>|manifest|report}" >&2
    exit 2
    ;;
esac
