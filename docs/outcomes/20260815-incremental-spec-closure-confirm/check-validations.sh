#!/usr/bin/env bash
# Asserts the four full-spec /smelt:validate closure reports exist at
# docs/validations/2026-09-04-<slug>-closure.md, each carries the eight
# /smelt:validate report sections, and that no "❌" line survives without a
# trailing disposition marker: "— fixed this phase", "— phase row <N>",
# "— blocked", or "— flagged-open: <ID>".
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DIR/../../.." && pwd)"
VALIDATIONS_DIR="$REPO_ROOT/docs/validations"

slugs=(definition_deltas incremental_models incremental_shapes model_properties)

required_sections=(
  "Automated checks"
  "Surface drift"
  "Semantics drift"
  "Invariant drift"
  "Timeless-oracle drift"
  "Freshness"
  "Summary"
)

fail=0

for slug in "${slugs[@]}"; do
  report="$VALIDATIONS_DIR/2026-09-04-${slug}-closure.md"
  if [ ! -f "$report" ]; then
    echo "FAIL: missing report $report" >&2
    fail=1
    continue
  fi

  for section in "${required_sections[@]}"; do
    if ! grep -qF "$section" "$report"; then
      echo "FAIL: $report missing required section '$section'" >&2
      fail=1
    fi
  done

  while IFS= read -r line; do
    if ! grep -qE -- '— (fixed this phase|phase row [0-9]+|blocked|flagged-open: [A-Za-z0-9-]+)' <<<"$line"; then
      echo "FAIL: $report has an undispositioned ❌ line: $line" >&2
      fail=1
    fi
  done < <(grep -F '❌' "$report" || true)
done

if [ "$fail" -ne 0 ]; then
  exit 1
fi

echo "OK: all four closure reports present with required sections and dispositioned ❌ lines"
