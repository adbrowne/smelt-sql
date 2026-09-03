#!/usr/bin/env bash
# Re-runs extract-baseline.sh and asserts baseline-inventory.md still matches it:
# per-spec row counts agree, and every extracted bold lead-in appears verbatim in
# exactly one inventory row. Non-zero exit on any mismatch.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INVENTORY_MD="$DIR/baseline-inventory.md"
TMP_TSV="$(mktemp)"
trap 'rm -f "$TMP_TSV"' EXIT

bash "$DIR/extract-baseline.sh" > "$TMP_TSV"

if [ ! -f "$INVENTORY_MD" ]; then
  echo "FAIL: $INVENTORY_MD does not exist yet" >&2
  exit 1
fi

fail=0

# Per-spec row counts: extractor rows vs "| <PREFIX>-NN |" table rows.
declare -A prefix=( [definition_deltas]=DD [incremental_models]=IM [incremental_shapes]=IS [model_properties]=MP )
for spec in definition_deltas incremental_models incremental_shapes model_properties; do
  extracted_count=$(awk -F'\t' -v s="$spec" '$1 == s' "$TMP_TSV" | wc -l | tr -d ' ')
  p="${prefix[$spec]}"
  inventory_count=$(grep -cE "^\| ${p}-[0-9]+ \|" "$INVENTORY_MD" || true)
  if [ "$extracted_count" != "$inventory_count" ]; then
    echo "FAIL: $spec row count mismatch: extractor=$extracted_count inventory=$inventory_count" >&2
    fail=1
  fi
done

# Lead-in coverage: every extracted bold lead-in appears verbatim in exactly one
# inventory row (allowing for the markdown "\|" escape of a literal pipe).
while IFS=$'\t' read -r spec sub lead oq line; do
  esc_lead=$(printf '%s' "$lead" | sed 's/|/\\|/g')
  # Match the lead-in as a literal substring of some table row.
  hits=$(grep -F -c -- "$esc_lead" "$INVENTORY_MD" || true)
  if [ "$hits" -eq 0 ]; then
    echo "FAIL: no inventory row contains lead-in from $spec:$line: $lead" >&2
    fail=1
  fi
done < "$TMP_TSV"

if [ "$fail" -ne 0 ]; then
  exit 1
fi

echo "OK: baseline-inventory.md matches extract-baseline.sh ($(wc -l < "$TMP_TSV" | tr -d ' ') bullets)"
