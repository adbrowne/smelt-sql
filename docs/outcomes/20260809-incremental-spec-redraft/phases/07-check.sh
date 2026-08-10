#!/usr/bin/env bash
# Phase 7 red-green checks for retiring `nondeterministic_columns` (the list-form
# non-determinism opt-in), replaced outright by `columns.<c>.contract: plausible`.
# Run by hand from the repo root: bash docs/outcomes/20260809-incremental-spec-redraft/phases/07-check.sh
set -uo pipefail

FAIL=0

pass() { echo "PASS: $1"; }
fail() { echo "FAIL: $1"; FAIL=1; }

# 1. no_live_struct_field — no production struct DEFINITION exposes a bare
#    `nondeterministic_columns` field (only the renamed `nondeterministic_columns_retired`
#    sentinel survives). Every remaining `nondeterministic_columns` identifier in
#    Rust source is either that sentinel, a local var/fn describing the retirement
#    (`raw_nondeterministic_columns`, `reject_nondeterministic_columns`), a doc
#    comment/string literal narrating the retired surface, or a test's YAML fixture
#    proving the key is refused.
no_live_struct_field_check() {
  local hits
  hits=$(rg -n '^\s*(pub )?nondeterministic_columns\s*:\s*Vec' --type rust || true)
  if [[ -z "$hits" ]]; then
    pass "no_live_struct_field"
  else
    fail "no_live_struct_field (hits:
$hits)"
  fi
}
no_live_struct_field_check

# 2. retirement_sentinel_present — the renamed sentinel field exists and is wired
#    to a `deserialize_with` rejector.
retirement_sentinel_check() {
  local bad=0
  grep -q 'nondeterministic_columns_retired' crates/smelt-core/src/config.rs \
    || { fail "retirement_sentinel: field not found in config.rs"; bad=1; }
  grep -q 'deserialize_with = "reject_nondeterministic_columns"' crates/smelt-core/src/config.rs \
    || { fail "retirement_sentinel: deserialize_with wiring not found"; bad=1; }
  [[ $bad -eq 0 ]] && pass "retirement_sentinel_present"
}
retirement_sentinel_check

# 3. spec_docs_retirement_only — every `docs/specs/` paragraph mentioning
#    `nondeterministic_columns` also mentions `columns.<c>.contract` somewhere in the
#    same paragraph (blank-line-delimited block) — i.e. it is retirement/fix-it
#    framing, never a standalone living declaration.
paragraph_pairing_check() {
  local dir="$1"
  local label="$2"
  local bad=0
  local file
  while IFS= read -r file; do
    local para="" found_target=0 found_pair=0
    flush() {
      if [[ $found_target -eq 1 && $found_pair -eq 0 ]]; then
        fail "$label: paragraph in $file mentions nondeterministic_columns without columns.<c>.contract pairing"
        bad=1
      fi
      found_target=0
      found_pair=0
    }
    while IFS= read -r line; do
      if [[ -z "$line" ]]; then
        flush
        continue
      fi
      if echo "$line" | grep -q 'nondeterministic_columns'; then
        found_target=1
      fi
      if echo "$line" | grep -qE 'columns\.<c>\.contract|columns\.\{column\}\.contract|columns\.c\.contract'; then
        found_pair=1
      fi
    done < "$file"
    flush
  done < <(grep -rl 'nondeterministic_columns' "$dir" 2>/dev/null || true)
  [[ $bad -eq 0 ]] && pass "$label"
}
paragraph_pairing_check "docs/specs" "spec_docs_retirement_only"
paragraph_pairing_check "docs-site/docs" "docs_site_retirement_only"

# 4. no_known_divergence_gap — model_properties.md's Known Divergences no longer
#    carries the "list-form declaration is not yet removed from the parser" gap
#    bullet this phase closes.
no_known_divergence_gap_check() {
  local hits
  hits=$(grep -n "list-form declaration is not yet removed from the parser" \
    docs/specs/model_properties.md || true)
  if [[ -z "$hits" ]]; then
    pass "no_known_divergence_gap"
  else
    fail "no_known_divergence_gap (hits:
$hits)"
  fi
}
no_known_divergence_gap_check

# 5. plausible_contract_skeleton_diagnostic — the new diagnostic code is registered
#    in diagnostics.md and defined in smelt-core/smelt-db.
plausible_contract_skeleton_diagnostic_check() {
  local bad=0
  grep -q 'PlausibleContractOnSkeletonColumn' docs/specs/diagnostics.md \
    || { fail "plausible_contract_skeleton_diagnostic: not in docs/specs/diagnostics.md"; bad=1; }
  grep -q 'PlausibleContractOnSkeletonColumn' crates/smelt-core/src/metadata.rs \
    || { fail "plausible_contract_skeleton_diagnostic: not in smelt-core/src/metadata.rs"; bad=1; }
  grep -q 'PlausibleContractOnSkeletonColumn' crates/smelt-db/src/diagnostics_types.rs \
    || { fail "plausible_contract_skeleton_diagnostic: not in smelt-db/src/diagnostics_types.rs"; bad=1; }
  [[ $bad -eq 0 ]] && pass "plausible_contract_skeleton_diagnostic"
}
plausible_contract_skeleton_diagnostic_check

# 6. timeless — "Phase [A-Z0-9]" only ever on a line also carrying a docs/plans/ or
#    docs/outcomes/ link, in the spec files this phase edited.
timeless_check() {
  local bad=0
  for f in docs/specs/models.md docs/specs/model_properties.md docs/specs/incremental_models.md docs/specs/diagnostics.md; do
    while IFS= read -r line; do
      # Skip the boilerplate Timeless-oracle-rule callout itself (present
      # verbatim near the top of every spec, quoting the banned patterns as
      # examples) — not content this phase introduced.
      if echo "$line" | grep -q '^> \*\*Timeless-oracle rule\.\*\*'; then
        continue
      fi
      if echo "$line" | grep -qE 'Phase [A-Z0-9] '; then
        if ! echo "$line" | grep -qE 'docs/plans/|docs/outcomes/'; then
          fail "timeless:$f ($line)"
          bad=1
        fi
      fi
    done < "$f"
  done
  [[ $bad -eq 0 ]] && pass "timeless"
}
timeless_check

exit $FAIL
