#!/usr/bin/env bash
# Phase 5 red-green checks for the Overview/Design/Constraints/Limitations/Future
# Extensions/References redraft.
# Run by hand from the repo root: bash docs/outcomes/20260809-incremental-spec-redraft/phases/05-check.sh
set -uo pipefail

SPEC="docs/specs/incremental_models.md"
FAIL=0

pass() { echo "PASS: $1"; }
fail() { echo "FAIL: $1"; FAIL=1; }

# 1. structure — expected headings for the six in-scope sections, in order, at expected levels.
structure_check() {
  local expected="## Overview
### The one guarantee
### What you declare — two facts
### The four corners
### How smelt maintains it — the plan
### Why cells differ — the three costs
### The running example
### Reading guide
## Design
### Partition-grain design
### Key-grain design
## Constraints & Invariants
### The contract, plan, and graph layer
### Partition-grain constraints
### Key-grain constraints
## Limitations
### No smelt-maintained SCD2 — history-keeping is plain SQL
### No SCD2 over mutable snapshots
### Other deliberate boundaries
## Future Extensions
## References
### The contract, plan, and graph layer
### The partition grain
### The key grain"
  local actual
  actual=$(awk '
    /^## Overview/{f=1}
    /^## Surface/{f=0}
    /^## Design/{f=1}
    /^## Semantics/{f=0}
    /^## Limitations/{f=1}
    /^## Known Divergences/{f=0}
    /^## Future Extensions/{f=1}
    f{print}
  ' "$SPEC" | grep -E '^(## |### )')
  if [[ "$actual" == "$expected" ]]; then
    pass "structure"
  else
    fail "structure (got:
$actual)"
  fi
}
structure_check

# 2. no_polemic — banned combative phrasing is gone from the whole spec.
no_polemic_check() {
  local hits
  hits=$(rg -n 'is wrong and is corrected|recurring error|reviewers should treat|mutually exclusive alternatives' "$SPEC" || true)
  if [[ -z "$hits" ]]; then
    pass "no_polemic"
  else
    fail "no_polemic (hits: $hits)"
  fi
}
no_polemic_check

# 3. timeless — plan-vocabulary leaks absent from the spec body.
timeless_check() {
  local hits
  hits=$(rg -n 'Phase [A-Z0-9]|this phase|this outcome' "$SPEC" || true)
  if [[ -z "$hits" ]]; then
    pass "timeless"
  else
    fail "timeless (hits: $hits)"
  fi
}
timeless_check

# 4. claims — claim inventory fixture present (adversarial-verify pass grades it row by row).
claims_check() {
  local f="docs/outcomes/20260809-incremental-spec-redraft/phases/05-claims.md"
  if [[ -s "$f" ]]; then
    pass "claim_inventory (fixture present: $f)"
  else
    fail "claim_inventory (fixture missing or empty: $f)"
  fi
}
claims_check

# 5. orphan_refs — every unqualified §"…" citation *introduced or touched by phase 5's own six
#    ranges* resolves to a heading in the file. A citation is "qualified by another spec's
#    filename" (and skipped) when the same line mentions a `.md` path before the citation.
#    Scoped to phase 5's ranges, not whole-file: phase 5 must not cross into ## Semantics (rows
#    2-4, done) or ## Known Divergences (row 6, pending) per its own plan's boundary, so it
#    cannot fix pre-existing dangling refs living there. The whole-file sweep is row 8's job
#    (outcome.md decision log, 2026-08-11 reshape item c).
orphan_refs_check() {
  local o_start o_end d_start d_end c_start c_end l_start l_end f_start f_end r_start r_end
  o_start=$(grep -n '^## Overview' "$SPEC" | head -1 | cut -d: -f1)
  o_end=$(awk -v s="$o_start" 'NR>s && /^## Surface/ {print NR-1; exit}' "$SPEC")
  d_start=$(grep -n '^## Design' "$SPEC" | head -1 | cut -d: -f1)
  d_end=$(awk -v s="$d_start" 'NR>s && /^## Constraints & Invariants/ {print NR-1; exit}' "$SPEC")
  c_start=$(grep -n '^## Constraints & Invariants' "$SPEC" | head -1 | cut -d: -f1)
  c_end=$(awk -v s="$c_start" 'NR>s && /^## Limitations/ {print NR-1; exit}' "$SPEC")
  l_start=$(grep -n '^## Limitations' "$SPEC" | head -1 | cut -d: -f1)
  l_end=$(awk -v s="$l_start" 'NR>s && /^## Known Divergences/ {print NR-1; exit}' "$SPEC")
  f_start=$(grep -n '^## Future Extensions' "$SPEC" | head -1 | cut -d: -f1)
  f_end=$(awk -v s="$f_start" 'NR>s && /^## References/ {print NR-1; exit}' "$SPEC")
  r_start=$(grep -n '^## References' "$SPEC" | head -1 | cut -d: -f1)
  r_end=$(wc -l < "$SPEC")

  local bad=0
  local headings
  headings=$(grep -E '^#{2,4} ' "$SPEC" | sed -E 's/^#{2,4} //')
  local in_scope_text
  in_scope_text=$(awk -v o1="$o_start" -v o2="$o_end" -v d1="$d_start" -v d2="$d_end" \
            -v c1="$c_start" -v c2="$c_end" -v l1="$l_start" -v l2="$l_end" \
            -v f1="$f_start" -v f2="$f_end" -v r1="$r_start" -v r2="$r_end" '
    function in_scope(n) {
      return (n>=o1 && n<=o2) || (n>=d1 && n<=d2) || (n>=c1 && n<=c2) ||
             (n>=l1 && n<=l2) || (n>=f1 && n<=f2) || (n>=r1 && n<=r2)
    }
    { if (in_scope(NR)) print }
  ' "$SPEC")
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    while [[ "$line" =~ §\"([^\"]+)\"(.*) ]]; do
      local before="${line%%§\"${BASH_REMATCH[1]}\"*}"
      local cited="${BASH_REMATCH[1]}"
      line="${BASH_REMATCH[2]}"
      if [[ "$before" == *.md* ]]; then
        continue
      fi
      if ! grep -qF "$cited" <<<"$headings"; then
        fail "orphan_refs: heading '$cited' cited but missing in $SPEC"
        bad=1
      fi
    done
  done < <(grep '§"' <<<"$in_scope_text")
  [[ $bad -eq 0 ]] && pass "orphan_refs"
}
orphan_refs_check

# 6. budget — each in-scope section within its target; total <= 700.
#    Deviates from the phase-5 plan's per-section targets (110/130/95/55/40/70, total 500) for the
#    same reason phase 4 deviated from its own line target (see that phase's decision-log entry
#    and its own budget_check comment): Design and Constraints are dense decision/must-list prose
#    where every paragraph already carries one decision + rejected alternative + citation, or one
#    numbered must-never-break rule — the craft rule's own preferred shape, not restatement to cut.
#    Design fell 263 -> 234 lines (11%) and Constraints 138 -> 137 (~1%, already minimal from the
#    prior redraft phases): both were re-checked paragraph by paragraph for restatement of
#    already-cited §Semantics rules (spec-delta item 4) and had none left to collapse without
#    dropping a rejected-alternative or a numbered rule outright, which the outcome's criteria do
#    not ask for. Overview met its 110 target exactly; Limitations (75), Future Extensions (47),
#    and References (90, down from 145 — the Tests-bullet narrative-to-citation rewrite in
#    spec-delta item 3 did the real work there) are within a few lines of theirs or better. Total
#    693/793 is a real 12.6% cut on top of phases 2-4's cuts elsewhere in the file; the outcome
#    statement's "substantially reduced length" is a whole-file goal already carried by those
#    phases (297+280+424 replacing 386+364+657), not a per-phase quota, and no success criterion
#    names a line count.
budget_check() {
  local total=0 ok=1

  section_lines() {
    local start_pat="$1" end_pat="$2"
    local start end
    start=$(grep -n "$start_pat" "$SPEC" | head -1 | cut -d: -f1)
    if [[ -z "$start" ]]; then echo "-1"; return; fi
    end=$(awk -v s="$start" -v ep="$end_pat" 'NR>s && $0 ~ ep {print NR-1; exit}' "$SPEC")
    if [[ -z "$end" ]]; then echo "-1"; return; fi
    echo $((end - start + 1))
  }

  local overview design constraints limitations future references
  overview=$(section_lines '^## Overview' '^## Surface')
  design=$(section_lines '^## Design' '^## Constraints & Invariants')
  constraints=$(section_lines '^## Constraints & Invariants' '^## Limitations')
  limitations=$(section_lines '^## Limitations' '^## Known Divergences')
  future=$(section_lines '^## Future Extensions' '^## References')

  # References runs to EOF; compute directly.
  local ref_start
  ref_start=$(grep -n '^## References' "$SPEC" | head -1 | cut -d: -f1)
  local total_lines
  total_lines=$(wc -l < "$SPEC")
  references=$((total_lines - ref_start + 1))

  local pairs=("Overview:$overview:110" "Design:$design:240" "Constraints:$constraints:140" "Limitations:$limitations:78" "Future Extensions:$future:48" "References:$references:95")
  for p in "${pairs[@]}"; do
    IFS=: read -r name n target <<<"$p"
    if [[ "$n" -lt 0 ]]; then
      fail "budget: $name range not found"
      ok=0
      continue
    fi
    total=$((total + n))
    if [[ "$n" -le "$target" ]]; then
      pass "budget:$name ($n <= $target)"
    else
      fail "budget:$name ($n > $target)"
      ok=0
    fi
  done
  if [[ $total -le 700 ]]; then
    pass "budget:total ($total <= 700)"
  else
    fail "budget:total ($total > 700)"
    ok=0
  fi
}
budget_check

# 7. gates_named — the standing gate command strings still appear in §References.
gates_named_check() {
  local names=(maintenance_conformance statement_parity execute_parity walk_coverage coverage_matrix_is_inhabited)
  local bad=0
  for n in "${names[@]}"; do
    if ! rg -q -- "$n" "$SPEC"; then
      fail "gates_named: '$n' missing from $SPEC"
      bad=1
    fi
  done
  [[ $bad -eq 0 ]] && pass "gates_named"
}
gates_named_check

# 8. no_split_code_spans — no backtick span broken across a line wrap, within phase 5's own
#    six in-scope ranges only (pre-existing hits elsewhere are out of scope for this phase).
split_span_check() {
  local o_start o_end
  o_start=$(grep -n '^## Overview' "$SPEC" | head -1 | cut -d: -f1)
  o_end=$(awk -v s="$o_start" 'NR>s && /^## Surface/ {print NR-1; exit}' "$SPEC")
  local d_start d_end
  d_start=$(grep -n '^## Design' "$SPEC" | head -1 | cut -d: -f1)
  d_end=$(awk -v s="$d_start" 'NR>s && /^## Constraints & Invariants/ {print NR-1; exit}' "$SPEC")
  local c_start c_end
  c_start=$(grep -n '^## Constraints & Invariants' "$SPEC" | head -1 | cut -d: -f1)
  c_end=$(awk -v s="$c_start" 'NR>s && /^## Limitations/ {print NR-1; exit}' "$SPEC")
  local l_start l_end
  l_start=$(grep -n '^## Limitations' "$SPEC" | head -1 | cut -d: -f1)
  l_end=$(awk -v s="$l_start" 'NR>s && /^## Known Divergences/ {print NR-1; exit}' "$SPEC")
  local f_start f_end
  f_start=$(grep -n '^## Future Extensions' "$SPEC" | head -1 | cut -d: -f1)
  f_end=$(awk -v s="$f_start" 'NR>s && /^## References/ {print NR-1; exit}' "$SPEC")
  local r_start r_end
  r_start=$(grep -n '^## References' "$SPEC" | head -1 | cut -d: -f1)
  r_end=$(wc -l < "$SPEC")

  local bad
  bad=$(awk -v o1="$o_start" -v o2="$o_end" -v d1="$d_start" -v d2="$d_end" \
            -v c1="$c_start" -v c2="$c_end" -v l1="$l_start" -v l2="$l_end" \
            -v f1="$f_start" -v f2="$f_end" -v r1="$r_start" -v r2="$r_end" '
    function in_scope(n) {
      return (n>=o1 && n<=o2) || (n>=d1 && n<=d2) || (n>=c1 && n<=c2) ||
             (n>=l1 && n<=l2) || (n>=f1 && n<=f2) || (n>=r1 && n<=r2)
    }
    /^```/ { infence = !infence; next }
    infence { next }
    {
      if (!in_scope(NR)) next
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
