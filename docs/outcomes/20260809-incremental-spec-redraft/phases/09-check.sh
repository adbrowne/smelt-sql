#!/usr/bin/env bash
# Phase 9 red-green checks for retiring the `smelt.yml` `models.<name>.batched:`
# sub-block, replaced by top-level `merge_key:` (+ existing `safety_overrides:`).
# Run by hand from the repo root: bash docs/outcomes/20260809-incremental-spec-redraft/phases/09-check.sh
set -uo pipefail

FAIL=0

pass() { echo "PASS: $1"; }
fail() { echo "FAIL: $1"; FAIL=1; }

# 1. no_live_config_batched_field — `ModelConfig` (crates/smelt-core/src/config.rs)
#    no longer exposes a bare `batched: Option<PartitionGrainConfig>` field; only
#    the renamed `batched_retired: ()` sentinel survives. `ModelMetadata::batched`
#    (the internal frontmatter-side fold representation, never directly
#    deserialized from user YAML) is untouched by this phase and is excluded.
no_live_config_batched_field_check() {
  local hits
  hits=$(rg -n '^\s*pub batched\s*:\s*Option<PartitionGrainConfig>' crates/smelt-core/src/config.rs || true)
  if [[ -z "$hits" ]]; then
    pass "no_live_config_batched_field"
  else
    fail "no_live_config_batched_field (hits:
$hits)"
  fi
}
no_live_config_batched_field_check

# 2. retirement_sentinel_wired — the renamed sentinel field exists on ModelConfig
#    and is wired to a `deserialize_with` rejector; `merge_key` exists alongside it.
retirement_sentinel_check() {
  local bad=0
  grep -q 'pub batched_retired: ()' crates/smelt-core/src/config.rs \
    || { fail "retirement_sentinel: batched_retired field not found in config.rs"; bad=1; }
  grep -q 'deserialize_with = "reject_batched_subblock"' crates/smelt-core/src/config.rs \
    || { fail "retirement_sentinel: deserialize_with wiring not found"; bad=1; }
  grep -q 'pub merge_key: Option<Vec<String>>' crates/smelt-core/src/config.rs \
    || { fail "retirement_sentinel: ModelConfig::merge_key not found"; bad=1; }
  grep -q 'pub merge_key: Option<Vec<String>>' crates/smelt-core/src/metadata.rs \
    || { fail "retirement_sentinel: ModelMetadata::merge_key not found"; bad=1; }
  grep -q 'fn fold_top_level_merge_key' crates/smelt-core/src/metadata.rs \
    || { fail "retirement_sentinel: fold_top_level_merge_key not found"; bad=1; }
  grep -q '"merge_key", &\[DeclarationKind::Model\]' crates/smelt-core/src/frontmatter.rs \
    || { fail "retirement_sentinel: merge_key missing from the frontmatter key catalogue"; bad=1; }
  [[ $bad -eq 0 ]] && pass "retirement_sentinel_wired"
}
retirement_sentinel_check

# 3. spec_docs_retirement_paired — every `docs/specs/` and `docs-site/docs/`
#    paragraph (blank-line-delimited block) mentioning `batched.unique_key`
#    (the specific sub-key this phase retires) also mentions `merge_key`
#    somewhere in the same paragraph — i.e. it is retirement/fix-it framing
#    naming the actual replacement, never a standalone living declaration.
#    Generic `` `batched:` `` mentions (the whole-block retirement, already
#    covered by phase 7/8) are not required to re-pair with `merge_key`.
paragraph_pairing_check() {
  local dir="$1"
  local label="$2"
  local bad=0
  local file
  while IFS= read -r file; do
    local found_target=0 found_pair=0
    flush() {
      if [[ $found_target -eq 1 && $found_pair -eq 0 ]]; then
        fail "$label: paragraph in $file mentions batched.unique_key without merge_key pairing"
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
      if echo "$line" | grep -q 'batched\.unique_key'; then
        found_target=1
      fi
      if echo "$line" | grep -q 'merge_key'; then
        found_pair=1
      fi
    done < "$file"
    flush
  done < <(grep -rl 'batched\.unique_key' "$dir" 2>/dev/null || true)
  [[ $bad -eq 0 ]] && pass "$label"
}
paragraph_pairing_check "docs/specs" "spec_docs_retirement_paired"
paragraph_pairing_check "docs-site/docs" "docs_site_retirement_paired"

# 4. merge_key_documented — merge_key: is documented in the smelt.yml reference.
merge_key_documented_check() {
  if grep -q '`merge_key`' docs-site/docs/reference/smelt-yml.md; then
    pass "merge_key_documented"
  else
    fail "merge_key_documented: no \`merge_key\` mention in docs-site/docs/reference/smelt-yml.md"
  fi
}
merge_key_documented_check

# 5. no_smelt_yml_batched_fixtures — no Rust test fixture still declares the
#    smelt.yml `batched:` sub-block as a *working* config (only as an
#    expected-refusal fixture, which the paragraph-pairing style check above
#    does not cover since it scans docs, not Rust). Every remaining
#    `batched:\n      unique_key:`-shaped generated smelt.yml string must be
#    gone in favour of `merge_key:`.
no_smelt_yml_batched_fixtures_check() {
  local hits
  hits=$(rg -n 'models:\\n\s+\S+:\\n\s+batched:\\n\s+unique_key' --type rust -g '!target' || true)
  if [[ -z "$hits" ]]; then
    pass "no_smelt_yml_batched_fixtures"
  else
    fail "no_smelt_yml_batched_fixtures (hits:
$hits)"
  fi
}
no_smelt_yml_batched_fixtures_check

# 6. timeless — "Phase [A-Z0-9]" only ever on a line also carrying a docs/plans/ or
#    docs/outcomes/ link, in the spec files this phase edited.
timeless_check() {
  local bad=0
  for f in docs/specs/models.md docs/specs/incremental_models.md docs/specs/diagnostics.md; do
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
