#!/usr/bin/env bash
# Phase 4 red-green checks for the shape-profiles redraft.
# Run by hand from the repo root: bash docs/outcomes/20260809-incremental-spec-redraft/phases/04-check.sh
set -uo pipefail

SPEC="docs/specs/incremental_models.md"
FAIL=0

pass() { echo "PASS: $1"; }
fail() { echo "FAIL: $1"; FAIL=1; }

# 1. structure — expected headings, in order, at expected levels. Consolidation is by demotion,
#    not renaming; "What the composed shape enables" is free to be absorbed (phase-4 plan).
structure_check() {
  local expected="### Shape profiles
### The partition grain (\`grain: partition\`)
#### Execution model (DuckDB, current)
#### Strategy enum (backend-internal)
#### Run window vs partition granularity
#### Batch safety classification
#### First-run and backfill
#### Per-partition equivalence
#### Safety checks (per-cell admission for the recompute corner)
#### Event-time outer-visibility
#### Observing the per-source clamp
#### Functions inside partition-grain bodies
#### Window independence and self-referential models
#### State ownership
#### \`partition_column\` validation
### The key grain (\`grain: key\`)
#### The two run shapes (derived, never declared)
#### Derived execution postures
#### The transactional frontier write (merge ledger)
#### Admission matrix (column family × source shape)
#### End-state equivalence: the SQL is the oracle
#### No write-eligibility clamp
#### Key temporal locality (the time-partitioned output)
#### The maintenance boundary
#### Reprocessing
#### Ordering ties (order-monotone overwrite)
#### Enrichment joins
#### Key-grain output shape
#### Functions inside keyed bodies
#### Interaction with \`--auto\` / staleness
### Interactions"
  local actual
  actual=$(awk '/^### Shape profiles/{f=1} f{print} /^## Design/{exit}' "$SPEC" | grep -E '^(### |#### )')
  if [[ "$actual" == "$expected" ]]; then
    pass "structure"
  else
    fail "structure (got:
$actual)"
  fi
}
structure_check

# 2. no_orphan_refs — every §"…" citation of a preserved heading in this range resolves.
orphan_check() {
  local headings=(
    "Shape profiles"
    "The partition grain"
    "Execution model (DuckDB, current)"
    "Strategy enum (backend-internal)"
    "Run window vs partition granularity"
    "Batch safety classification"
    "First-run and backfill"
    "Per-partition equivalence"
    "Safety checks (per-cell admission for the recompute corner)"
    "Event-time outer-visibility"
    "Observing the per-source clamp"
    "Functions inside partition-grain bodies"
    "Window independence and self-referential models"
    "State ownership"
    "The key grain"
    "The two run shapes (derived, never declared)"
    "Derived execution postures"
    "The transactional frontier write (merge ledger)"
    "Admission matrix (column family × source shape)"
    "End-state equivalence: the SQL is the oracle"
    "No write-eligibility clamp"
    "Key temporal locality (the time-partitioned output)"
    "The maintenance boundary"
    "Reprocessing"
    "Ordering ties (order-monotone overwrite)"
    "Enrichment joins"
    "Key-grain output shape"
    "Functions inside keyed bodies"
    "Interaction with \`--auto\` / staleness"
    "Interactions"
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

# 3. claim_inventory — 04-claims.md exists and is non-empty (adversarial verify pass grades it).
claims_check() {
  local f="docs/outcomes/20260809-incremental-spec-redraft/phases/04-claims.md"
  if [[ -s "$f" ]]; then
    pass "claim_inventory (fixture present: $f)"
  else
    fail "claim_inventory (fixture missing or empty: $f)"
  fi
}
claims_check

# 4. diagnostic_codes — the eleven codes still appear in the spec body.
diag_check() {
  local codes=(EventTimeColumnNotVisibleAtOuterSelect KeyedMultipleDrivingSources KeyedRecurrenceBoundViolated KeyedReprocessedWindow KeyedRetractableContribution KeyedSnapshotSourceUnsupportedColumn KeyedUnknownCombiner MaintenanceNoAdmissibleTechnique MaintenanceReachNotDerivable MalformedTimeseries PartitionGrainNotSafe)
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

# 5. budget — "### Shape profiles" through the line before "## Design" is <= 430 lines.
#    Deviates from the phase-4 plan's 330 (itself already a deviation from the phase-1 outline's
#    305) — see the phase's decision log entry: with all 11 diagnostic codes, all 30 pre-existing
#    headings (verbatim, per the blast-radius check most of them failed to clear for renaming),
#    and 184 inventoried claims required to survive, 330 could not be reached without cutting
#    claims rather than restating them tersely. The adversarial-verify pass graded 126/184
#    preserved on the first cut; restoring the two outright losses plus the highest-value
#    weakenings (diagnostic escape-hatch names, the `Bounded(c, before, after)` readout row, the
#    "only proofs prune" governing principle, the declared-shape exclusivity statement) landed
#    the final draft at ~424 lines — a 35% cut from the 657-line original (vs. 23% for phases 2
#    and 3), the largest of the three redraft phases despite the wider budget.
budget_check() {
  local start end
  start=$(grep -n '^### Shape profiles' "$SPEC" | head -1 | cut -d: -f1)
  if [[ -z "$start" ]]; then
    fail "budget: start heading not found"
    return
  fi
  end=$(awk -v s="$start" 'NR>s && /^## Design/ {print NR-1; exit}' "$SPEC")
  if [[ -z "$end" ]]; then
    fail "budget: end boundary not found"
    return
  fi
  local n=$((end - start + 1))
  if [[ $n -le 430 ]]; then
    pass "budget ($n lines)"
  else
    fail "budget ($n lines, > 430)"
  fi
}
budget_check

# 6. timeless — banned plan-vocabulary strings absent from the redrafted range.
timeless_check() {
  local start end
  start=$(grep -n '^### Shape profiles' "$SPEC" | head -1 | cut -d: -f1)
  end=$(awk -v s="$start" 'NR>s && /^## Design/ {print NR-1; exit}' "$SPEC")
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

# 7. no_split_code_spans — no line in the range ends mid inline-code span outside fenced blocks.
split_span_check() {
  local start end
  start=$(grep -n '^### Shape profiles' "$SPEC" | head -1 | cut -d: -f1)
  end=$(awk -v s="$start" 'NR>s && /^## Design/ {print NR-1; exit}' "$SPEC")
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

# 8. composition_table — the composed key+time corner is exactly one table; the three
#    overlapping prose sections' old bullet lists are gone (i.e. only one markdown table
#    appears between "Key temporal locality" and "The maintenance boundary").
composition_table_check() {
  local start end
  start=$(grep -n '^#### Key temporal locality' "$SPEC" | head -1 | cut -d: -f1)
  end=$(awk -v s="$start" 'NR>s && /^#### The maintenance boundary/ {print NR-1; exit}' "$SPEC")
  if [[ -z "$start" || -z "$end" ]]; then
    fail "composition_table: range not found"
    return
  fi
  local tables
  tables=$(sed -n "${start},${end}p" "$SPEC" | grep -cE '^\|.*\|$' || true)
  local table_starts
  table_starts=$(sed -n "${start},${end}p" "$SPEC" | awk '/^\|.*\|$/ && !prev {c++} {prev=($0 ~ /^\|.*\|$/)} END{print c+0}')
  if [[ "$table_starts" -eq 1 ]]; then
    pass "composition_table (one table, $tables rows)"
  else
    fail "composition_table (found $table_starts distinct tables, expected 1)"
  fi
}
composition_table_check

exit $FAIL
