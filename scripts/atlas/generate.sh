#!/usr/bin/env bash
# Regenerate the Maintenance Atlas (crates/smelt-maintenance-testkit/examples/stage_atlas.rs
# + examples/{timeseries,web_analytics,retail_analytics} -> a single self-contained HTML
# explorer of derived maintenance plans) and smoke-test it.
#
# Usage: bash scripts/atlas/generate.sh [out_html_path]
#
# Requires DUCKDB_LIB_DIR to be set (see CLAUDE.md). Not published anywhere by
# this script — it just writes the HTML file and exits nonzero on failure, so
# it composes as a CI correctness gate. Pass an out path under a git-ignored
# dir (default: target/atlas/atlas.html) or upload it as a CI artifact.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

OUT="${1:-$ROOT/target/atlas/atlas.html}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

mkdir -p "$(dirname "$OUT")"

echo "== building smelt CLI =="
cargo build -p smelt-cli --no-default-features --features duckdb --quiet

echo "== staging testkit recipe specimens =="
cargo run -p smelt-maintenance-testkit --example stage_atlas --quiet -- "$WORK/recipes"

echo "== harvesting smelt explain --json =="
BRANCH="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)"
python3 scripts/atlas/harvest.py \
  --smelt-bin "$ROOT/target/debug/smelt" \
  --repo-root "$ROOT" \
  --recipes-dir "$WORK/recipes" \
  --branch "$BRANCH" \
  --out "$WORK/atlas_data.json"

echo "== enriching with text-report extras =="
python3 scripts/atlas/enrich.py \
  --in "$WORK/atlas_data.json" \
  --out "$WORK/atlas_data_enriched.json"

echo "== combining into $OUT =="
python3 scripts/atlas/combine.py \
  --template scripts/atlas/template.html \
  --data "$WORK/atlas_data_enriched.json" \
  --app-js scripts/atlas/app.js \
  --out "$OUT"

echo "== smoke-testing =="
node scripts/atlas/smoke.js "$OUT"

echo "atlas written to $OUT"
