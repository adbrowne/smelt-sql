#!/usr/bin/env bash
# Verifies closure-report.md is complete per success criterion 1 and the
# phase-5 plan's test list:
#   - report_exists
#   - every_baseline_id_enumerated: every ID derived from baseline-inventory.tsv
#     (DD-01.., IM-01.., IS-01.., MP-01..; 80 total) appears in the report's
#     disposition table exactly once
#   - closed_ids_cite_a_sha: every row dispositioned `closed` names an 8+-hex sha
#   - open_ids_state_a_reason: every row dispositioned `open` carries a
#     non-empty reason cell
#   - all_six_criteria_sectioned: one section per success criterion 1-6
#   - criterion_5_gates_all_named: all five criterion-5 gate commands appear
#     with a PASS/FAIL verdict
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPORT="$DIR/closure-report.md"
TSV="$DIR/baseline-inventory.tsv"

if [ ! -f "$REPORT" ]; then
  echo "FAIL: report_exists — $REPORT does not exist yet" >&2
  exit 1
fi

fail=0

# Derive expected 80 IDs from baseline-inventory.tsv, numbered per-spec in file order.
declare -A prefix=( [definition_deltas]=DD [incremental_models]=IM [incremental_shapes]=IS [model_properties]=MP )
expected_ids=()
for spec in definition_deltas incremental_models incremental_shapes model_properties; do
  p="${prefix[$spec]}"
  n=$(awk -F'\t' -v s="$spec" '$1 == s' "$TSV" | wc -l | tr -d ' ')
  for i in $(seq 1 "$n"); do
    expected_ids+=("$(printf '%s-%02d' "$p" "$i")")
  done
done

if [ "${#expected_ids[@]}" -ne 80 ]; then
  echo "FAIL: every_baseline_id_enumerated — expected 80 derived IDs, got ${#expected_ids[@]}" >&2
  fail=1
fi

for id in "${expected_ids[@]}"; do
  hits=$(grep -cE "^\| ${id} \|" "$REPORT" || true)
  if [ "$hits" -ne 1 ]; then
    echo "FAIL: every_baseline_id_enumerated — $id appears $hits times in $REPORT (expected 1)" >&2
    fail=1
  fi
done

# closed_ids_cite_a_sha / open_ids_state_a_reason: scan the disposition table rows.
while IFS= read -r row; do
  row_noesc="${row//\\|/$'\x01'}"
  IFS='|' read -r _ id _spec _lead disposition evidence _ <<<"$row_noesc"
  id="$(echo "$id" | tr -d '[:space:]')"
  [[ "$id" =~ ^(DD|IM|IS|MP)-[0-9]+$ ]] || continue
  disposition="$(echo "$disposition" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
  evidence="$(echo "$evidence" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')"
  evidence="${evidence//$'\x01'/|}"

  first_word="${disposition%% *}"
  if [ "$first_word" = "closed" ]; then
    if ! grep -qE '[0-9a-f]{8,}' <<<"$disposition$evidence"; then
      echo "FAIL: closed_ids_cite_a_sha — $id disposition '$disposition' has no 8+-hex sha" >&2
      fail=1
    fi
  fi
  if [ "$first_word" = "open" ]; then
    if [ -z "$evidence" ]; then
      echo "FAIL: open_ids_state_a_reason — $id is 'open' but evidence/reason cell is empty" >&2
      fail=1
    fi
  fi
done < <(grep -E '^\| (DD|IM|IS|MP)-[0-9]+ \|' "$REPORT")

# all_six_criteria_sectioned
for n in 1 2 3 4 5 6; do
  if ! grep -qE "^#+ .*Criterion $n\b" "$REPORT"; then
    echo "FAIL: all_six_criteria_sectioned — no section header found for Criterion $n" >&2
    fail=1
  fi
done

# criterion_5_gates_all_named
gates=(
  "verify-phase.sh"
  "maintenance_conformance"
  "statement_parity"
  "walk_coverage"
  "execute_parity"
)
for gate in "${gates[@]}"; do
  if ! grep -qF -- "$gate" "$REPORT"; then
    echo "FAIL: criterion_5_gates_all_named — gate '$gate' not named in report" >&2
    fail=1
  fi
  if ! grep -F -- "$gate" "$REPORT" | grep -qE 'PASS|FAIL'; then
    echo "FAIL: criterion_5_gates_all_named — gate '$gate' has no PASS/FAIL verdict nearby" >&2
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  exit 1
fi

echo "OK: closure-report.md enumerates all 80 IDs with valid dispositions, all six criteria sectioned, all five gates verdicted"
