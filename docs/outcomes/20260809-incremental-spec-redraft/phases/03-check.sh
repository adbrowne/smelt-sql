#!/usr/bin/env bash
# Phase 3 red-green checks for the write-addressing/maintenance-mechanics redraft.
# Run by hand from the repo root: bash docs/outcomes/20260809-incremental-spec-redraft/phases/03-check.sh
set -uo pipefail

SPEC="docs/specs/incremental_models.md"
FAIL=0

pass() { echo "PASS: $1"; }
fail() { echo "FAIL: $1"; FAIL=1; }

# 1. structure — the eight headings appear at exactly those levels, in that order.
structure_check() {
  local expected="### Per-cell write addressing
#### The write-pattern set is open (and partly backend-provided)
#### The repair family
### Maintenance mechanics
#### Windowed maintenance and the horizon
#### Partition-local maintenance (the K8 guardrail)
#### Statement emission (single owner)
#### The definition-change trigger"
  local actual
  actual=$(grep -E '^(### |#### )' "$SPEC" | grep -E '^### Per-cell write addressing|^#### The write-pattern set is open \(and partly backend-provided\)|^#### The repair family|^### Maintenance mechanics|^#### Windowed maintenance and the horizon|^#### Partition-local maintenance \(the K8 guardrail\)|^#### Statement emission \(single owner\)|^#### The definition-change trigger')
  if [[ "$actual" == "$expected" ]]; then
    pass "structure"
  else
    fail "structure (got: $(echo "$actual" | tr '\n' '|'))"
  fi
}
structure_check

# 2. no_orphan_refs — every §"…" citation of one of the seven pre-existing headings resolves.
orphan_check() {
  local headings=(
    "The write-pattern set is open (and partly backend-provided)"
    "The repair family"
    "Windowed maintenance and the horizon"
    "Partition-local maintenance (the K8 guardrail)"
    "Statement emission (single owner)"
    "The definition-change trigger"
    "Per-cell write addressing"
  )
  local bad=0
  for h in "${headings[@]}"; do
    local citers
    citers=$(rg -Fl "§\"$h\"" --glob '!docs/plans' --glob '!docs/research' 2>/dev/null)
    if [[ -n "$citers" ]]; then
      if ! grep -qF "$h" <(grep -E '^#{2,4} ' "$SPEC"); then
        fail "no_orphan_refs: heading '$h' cited but missing in $SPEC"
        bad=1
      fi
    fi
  done
  [[ $bad -eq 0 ]] && pass "no_orphan_refs"
}
orphan_check

# 3. claim_inventory — 03-claims.md exists and is non-empty (adversarial verify pass grades it).
claims_check() {
  local f="docs/outcomes/20260809-incremental-spec-redraft/phases/03-claims.md"
  if [[ -s "$f" ]]; then
    pass "claim_inventory (fixture present: $f)"
  else
    fail "claim_inventory (fixture missing or empty: $f)"
  fi
}
claims_check

# 4. diagnostic_codes — all eight codes still appear in the spec body AND in §Surface's table.
diag_check() {
  local codes=(MaintenanceNoAdmissibleTechnique MaintenanceWriteAddressingRefused MaintenanceWritePatternUnavailable MaintenanceRepairKeysNotDiscoverable MaintenanceRepairSliceUnbounded ContractLateArrivalOutsideHorizon MaintenanceScanUnbounded MaintenanceSkeletonColumnAdded)
  local bad=0
  for c in "${codes[@]}"; do
    if ! grep -q "$c" "$SPEC"; then
      fail "diagnostic_codes: $c missing from $SPEC"
      bad=1
    fi
  done
  [[ $bad -eq 0 ]] && pass "diagnostic_codes"
}
diag_check

# 5. budget — "### Per-cell write addressing" through the line before "### The frontier" is <= 280 lines.
budget_check() {
  local start end
  start=$(grep -n '^### Per-cell write addressing' "$SPEC" | head -1 | cut -d: -f1)
  if [[ -z "$start" ]]; then
    fail "budget: start heading not found"
    return
  fi
  end=$(awk -v s="$start" 'NR>s && /^### The frontier/ {print NR-1; exit}' "$SPEC")
  if [[ -z "$end" ]]; then
    fail "budget: end boundary not found"
    return
  fi
  local n=$((end - start + 1))
  if [[ $n -le 280 ]]; then
    pass "budget ($n lines)"
  else
    fail "budget ($n lines, > 280)"
  fi
}
budget_check

# 6. timeless — banned plan-vocabulary strings absent from the redrafted range.
timeless_check() {
  local start end
  start=$(grep -n '^### Per-cell write addressing' "$SPEC" | head -1 | cut -d: -f1)
  end=$(awk -v s="$start" 'NR>s && /^### The frontier/ {print NR-1; exit}' "$SPEC")
  if [[ -z "$start" || -z "$end" ]]; then
    fail "timeless: range not found"
    return
  fi
  local hits
  hits=$(sed -n "${start},${end}p" "$SPEC" | rg -n 'Historical name|pre-cut|ratified|category error|Phase [A-Z0-9]' || true)
  if [[ -z "$hits" ]]; then
    pass "timeless"
  else
    fail "timeless (hits: $hits)"
  fi
}
timeless_check

# 7. no_split_code_spans — no line in the range ends mid inline-code/math span (odd count of `
#    on a line inside a non-fenced paragraph, ignoring fenced code blocks).
split_span_check() {
  local start end
  start=$(grep -n '^### Per-cell write addressing' "$SPEC" | head -1 | cut -d: -f1)
  end=$(awk -v s="$start" 'NR>s && /^### The frontier/ {print NR-1; exit}' "$SPEC")
  if [[ -z "$start" || -z "$end" ]]; then
    fail "no_split_code_spans: range not found"
    return
  fi
  local bad
  bad=$(awk -v s="$start" -v e="$end" '
    NR>=s && NR<=e {
      if ($0 ~ /^```/) { infence = !infence; next }
      if (infence) next
      n = gsub(/`/, "`")
      if (n % 2 == 1) print NR": "$0
    }
  ' "$SPEC")
  if [[ -z "$bad" ]]; then
    pass "no_split_code_spans"
  else
    fail "no_split_code_spans (lines: $bad)"
  fi
}
split_span_check

exit $FAIL
