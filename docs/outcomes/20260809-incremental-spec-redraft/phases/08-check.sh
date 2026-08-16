#!/usr/bin/env bash
# Phase 8 red-green checks for retiring declared `grain: key_per_partition` and the
# dead `IncrementalStrategy::{Append, InsertOverwrite}` variants.
# Run by hand from the repo root: bash docs/outcomes/20260809-incremental-spec-redraft/phases/08-check.sh
set -uo pipefail

FAIL=0

pass() { echo "PASS: $1"; }
fail() { echo "FAIL: $1"; FAIL=1; }

# 1. no_writable_key_per_partition — no spec or docs-site surface table still
#    offers `grain: key_per_partition` as something a modeller writes. This
#    greps for the literal declaration spelling; mentions of the *derived*
#    label ("derived `grain: key_per_partition`", "derived-only") are fine —
#    the check only fails on a bare `grain: key_per_partition` writable-set
#    listing (pipe-delimited enum or a plain `grain: key_per_partition` line
#    outside a code fence describing a fixture).
no_writable_key_per_partition_check() {
  local hits
  hits=$(rg -n 'partition \| key \| key_per_partition|key_per_partition\s*\|\s*validated against' \
    docs/specs docs-site/docs 2>/dev/null || true)
  if [[ -z "$hits" ]]; then
    pass "no_writable_key_per_partition"
  else
    fail "no_writable_key_per_partition (hits:
$hits)"
  fi
}
no_writable_key_per_partition_check

# 2. retirement_message_present — config.rs rejects the declaration with a
#    message naming the derived facts and grain: key.
retirement_message_check() {
  if grep -q 'grain: key_per_partition cannot be declared' crates/smelt-core/src/config.rs; then
    pass "retirement_message_present"
  else
    fail "retirement_message_present: rejection message not found in config.rs"
  fi
}
retirement_message_check

# 3. no_dead_strategy_variants — IncrementalStrategy has exactly one variant
#    (DeleteInsert); Append/InsertOverwrite are gone from crates/ production
#    and test code (docs/plans/ is a historical record and stays untouched).
no_dead_strategy_variants_check() {
  local hits
  hits=$(rg -n 'IncrementalStrategy::(Append|InsertOverwrite)|^\s*Append,\s*$|^\s*InsertOverwrite,\s*$' \
    crates/ 2>/dev/null || true)
  if [[ -z "$hits" ]]; then
    pass "no_dead_strategy_variants"
  else
    fail "no_dead_strategy_variants (hits:
$hits)"
  fi
}
no_dead_strategy_variants_check

# 4. backend_capability_survives — insert_into_from_query/insert_overwrite
#    stay on the Backend trait (the capability that would admit those
#    strategies later).
backend_capability_survives_check() {
  local bad=0
  grep -q 'fn insert_into_from_query' crates/smelt-backend/src/lib.rs \
    || { fail "backend_capability_survives: insert_into_from_query missing"; bad=1; }
  grep -q 'fn insert_overwrite' crates/smelt-backend/src/lib.rs \
    || { fail "backend_capability_survives: insert_overwrite missing"; bad=1; }
  [[ $bad -eq 0 ]] && pass "backend_capability_survives"
}
backend_capability_survives_check

# 5. kd_bullet_gone — the production-unreachable InsertOverwrite dead-code
#    Known-Divergence bullet is gone from incremental_models.md (the code it
#    described no longer exists).
kd_bullet_gone_check() {
  local hits
  hits=$(grep -n 'production-unreachable `InsertOverwrite`' docs/specs/incremental_models.md || true)
  if [[ -z "$hits" ]]; then
    pass "kd_bullet_gone"
  else
    fail "kd_bullet_gone (hits:
$hits)"
  fi
}
kd_bullet_gone_check

# 6. timeless — "Phase [A-Z0-9]" only ever on a line also carrying a
#    docs/plans/ or docs/outcomes/ link, in the spec files this phase edited.
timeless_check() {
  local bad=0
  for f in docs/specs/incremental_models.md docs/specs/models.md docs/specs/diagnostics.md docs/specs/architecture.md; do
    while IFS= read -r line; do
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
