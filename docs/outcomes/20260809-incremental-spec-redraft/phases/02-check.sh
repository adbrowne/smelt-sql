#!/usr/bin/env bash
# Phase 2 red-green checks for the typed-delta/lattice/plan-matrix redraft.
# Run by hand from the repo root: bash docs/outcomes/20260809-incremental-spec-redraft/phases/02-check.sh
set -uo pipefail

SPEC="docs/specs/incremental_models.md"
FAIL=0

pass() { echo "PASS: $1"; }
fail() { echo "FAIL: $1"; FAIL=1; }

# 1. structure — the seven headings exist at the listed levels in order.
structure_check() {
  local expected="### Typed deltas and the algebraic ladder
#### The equivalence invariant
#### The algebraic maintenance ladder
#### Decomposed state (rung 2) in keyed models
#### Validator, not chooser
### The contract lattice
### The plan matrix
#### Per-cell admission"
  local actual
  actual=$(grep -E '^(### |#### )' "$SPEC" | grep -E 'Typed deltas and the algebraic ladder|^#{3,4} The equivalence invariant|^#{3,4} The algebraic maintenance ladder|^#{3,4} Decomposed state \(rung 2\)|^#{3,4} Validator, not chooser|^### The contract lattice|^### The plan matrix|^#{3,4} Per-cell admission')
  if [[ "$actual" == "$expected" ]]; then
    pass "structure"
  else
    fail "structure (got: $(echo "$actual" | tr '\n' '|'))"
  fi
}
structure_check

# 2. no_orphan_refs — every §"…" citation of one of the seven headings (outside docs/plans, docs/research) resolves.
orphan_check() {
  local headings=(
    "The equivalence invariant"
    "The algebraic maintenance ladder"
    "Decomposed state (rung 2) in keyed models"
    "Validator, not chooser"
    "The contract lattice"
    "The plan matrix"
    "Per-cell admission"
  )
  local bad=0
  for h in "${headings[@]}"; do
    local citers
    citers=$(rg -l "§\"$h\"" --glob '!docs/plans' --glob '!docs/research' 2>/dev/null)
    if [[ -n "$citers" ]]; then
      if ! grep -qE "^#{2,4} $h\$" "$SPEC"; then
        fail "no_orphan_refs: heading '$h' cited but missing in $SPEC"
        bad=1
      fi
    fi
  done
  [[ $bad -eq 0 ]] && pass "no_orphan_refs"
}
orphan_check

# 3. claim_inventory — every claim number in 02-claims.md is referenced as covered (manual gate;
#    this script only checks the inventory file exists and is non-empty, the adversarial verify
#    pass (task 7) is what actually grades preserved/weakened/lost/strengthened).
claims_check() {
  local f="docs/outcomes/20260809-incremental-spec-redraft/phases/02-claims.md"
  if [[ -s "$f" ]]; then
    pass "claim_inventory (fixture present: $f)"
  else
    fail "claim_inventory (fixture missing or empty: $f)"
  fi
}
claims_check

# 4. diagnostic_codes — the six codes still appear in the spec body and in §Surface's diagnostics table.
diag_check() {
  local codes=(KeyedStateColumnCollision ContractLateArrivalOutsideHorizon ContractDeferralExceeded MaintenanceReachNotDerivable MaintenanceUnboundedFootprint MaintenanceRepairKeysNotDiscoverable)
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

# 5. budget — the redrafted range is <= 300 lines. Range = from the new group heading through the
#    end of "Per-cell admission" (up to the next top-level/section heading).
budget_check() {
  local start end
  start=$(grep -n '^### Typed deltas and the algebraic ladder' "$SPEC" | head -1 | cut -d: -f1)
  if [[ -z "$start" ]]; then
    fail "budget: start heading not found"
    return
  fi
  # end = line before "### Per-cell write addressing" (phase 3's section, the next boundary)
  end=$(awk -v s="$start" 'NR>s && /^### Per-cell write addressing/ {print NR-1; exit}' "$SPEC")
  if [[ -z "$end" ]]; then
    fail "budget: end boundary not found"
    return
  fi
  local n=$((end - start + 1))
  if [[ $n -le 300 ]]; then
    pass "budget ($n lines)"
  else
    fail "budget ($n lines, > 300)"
  fi
}
budget_check

# 6. timeless — banned plan-vocabulary strings absent from the redrafted range.
timeless_check() {
  local start end
  start=$(grep -n '^### Typed deltas and the algebraic ladder' "$SPEC" | head -1 | cut -d: -f1)
  end=$(awk -v s="$start" 'NR>s && /^### Per-cell write addressing/ {print NR-1; exit}' "$SPEC")
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

exit $FAIL
