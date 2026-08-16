#!/usr/bin/env bash
# Phase 10 red-green checks: whole-file citation sweep, docs-site terminology sync,
# timeless grep, and the 06-claims.md keep->drop reclassification.
# Run by hand from the repo root: bash docs/outcomes/20260809-incremental-spec-redraft/phases/10-check.sh
set -uo pipefail

IM="docs/specs/incremental_models.md"
MP="docs/specs/model_properties.md"
ARCH="docs/specs/architecture.md"
STATE="docs-site/docs/reference/state.md"
INCR="docs-site/docs/guide/incremental-models.md"
FAIL=0

pass() { echo "PASS: $1"; }
fail() { echo "FAIL: $1"; FAIL=1; }

# 1. orphan_refs_whole_file — every §"…" citation in both specs resolves to a real heading
#    (^#{2,4}) in the file it names on the same physical line; defaults to self-file when no
#    .md path precedes it on that line.
orphan_refs_whole_file_check() {
  local bad=0
  for f in "$IM" "$MP"; do
    local headings_self
    headings_self=$(grep -E '^#{2,4} ' "$f" | sed -E 's/^#{2,4} //')
    while IFS= read -r line; do
      [[ -z "$line" ]] && continue
      local work="$line"
      while [[ "$work" =~ §\"([^\"]+)\"(.*) ]]; do
        local cited="${BASH_REMATCH[1]}"
        local before="${work%%§\"${cited}\"*}"
        work="${BASH_REMATCH[2]}"
        local after="$work"
        # Self-file resolution wins first (a citation is presumed to name a heading in its own
        # file unless self-resolution fails). Only then fall back to an adjacent qualifier --
        # the .md path sitting immediately (whitespace/punctuation only) before OR after the
        # citation on the line. A `.md` mention elsewhere in the same long paragraph line does
        # not qualify a later, unrelated citation.
        if grep -qF "$cited" <<<"$headings_self"; then
          continue
        fi
        local before_trimmed="${before%"${before##*[![:space:]]}"}"
        local qualifier=""
        if [[ "$before_trimmed" =~ \`([A-Za-z0-9_./-]+\.md)\`[[:space:]]*$ ]]; then
          qualifier="${BASH_REMATCH[1]}"
        elif [[ "$after" =~ ^[,\ ]*\`([A-Za-z0-9_./-]+\.md)\` ]]; then
          qualifier="${BASH_REMATCH[1]}"
        fi
        local target_file="$f"
        if [[ -n "$qualifier" ]]; then
          target_file="$qualifier"
          [[ -f "$target_file" ]] || target_file="docs/specs/$qualifier"
        fi
        local headings
        if [[ "$target_file" == "$f" ]]; then
          headings="$headings_self"
        else
          headings=$(grep -E '^#{2,4} ' "$target_file" 2>/dev/null | sed -E 's/^#{2,4} //')
        fi
        if ! grep -qF "$cited" <<<"$headings"; then
          fail "orphan_refs_whole_file: '$cited' cited in $f -> $target_file, not found"
          bad=1
        fi
      done
    done < <(grep '§"' "$f")
  done
  [[ $bad -eq 0 ]] && pass "orphan_refs_whole_file"
}
orphan_refs_whole_file_check

# 2. citation_targets_are_files_that_exist — every .md path named alongside a §"…" citation
#    exists on disk.
citation_targets_are_files_that_exist_check() {
  local bad=0
  for f in "$IM" "$MP"; do
    while IFS= read -r qualifier; do
      [[ -z "$qualifier" ]] && continue
      local candidates=("docs/specs/$qualifier" "$qualifier")
      local found=0
      for c in "${candidates[@]}"; do
        [[ -f "$c" ]] && { found=1; break; }
      done
      if [[ $found -eq 0 ]]; then
        fail "citation_targets_are_files_that_exist: $qualifier (cited in $f) does not exist"
        bad=1
      fi
    done < <(grep -B0 '§"' "$f" | grep -oE '`[A-Za-z0-9_./-]+\.md`' | tr -d '`' | sort -u)
  done
  [[ $bad -eq 0 ]] && pass "citation_targets_are_files_that_exist"
}
citation_targets_are_files_that_exist_check

# 3. timeless_whole_file — plan vocabulary absent from both spec bodies (excluding the
#    Timeless-oracle boilerplate blockquote) and from the five listed docs-site pages.
timeless_whole_file_check() {
  local pattern='Phase [A-Z0-9]|this phase|this outcome'
  local hits
  hits=$(rg -n "$pattern" "$IM" "$MP" | rg -v 'Timeless-oracle rule' || true)
  if [[ -n "$hits" ]]; then
    fail "timeless_whole_file: spec body hits:
$hits"
  fi
  local docs_hits
  docs_hits=$(rg -n "$pattern" \
    docs-site/docs/guide/incremental-models.md \
    docs-site/docs/reference/state.md \
    docs-site/docs/reference/timeseries.md \
    docs-site/docs/reference/smelt-yml.md \
    docs-site/docs/guide/materializations.md 2>/dev/null || true)
  if [[ -n "$docs_hits" ]]; then
    fail "timeless_whole_file: docs-site hits:
$docs_hits"
  fi
  [[ -z "$hits" && -z "$docs_hits" ]] && pass "timeless_whole_file"
}
timeless_whole_file_check

# 4. docs_site_frontier_terminology — reference/state.md and guide/incremental-models.md each
#    mention "frontier" alongside "reconciliation ledger"; no docs-site page describes the
#    reconciliation ledger and the merge ledger as unrelated mechanisms.
docs_site_frontier_terminology_check() {
  local bad=0
  for f in "$STATE" "$INCR"; do
    if ! rg -qi 'frontier' "$f"; then
      fail "docs_site_frontier_terminology: $f does not mention 'frontier'"
      bad=1
    fi
    if ! rg -qi 'reconciliation ledger' "$f"; then
      fail "docs_site_frontier_terminology: $f does not mention 'reconciliation ledger'"
      bad=1
    fi
  done
  [[ $bad -eq 0 ]] && pass "docs_site_frontier_terminology"
}
docs_site_frontier_terminology_check

# 5. docs_site_no_retired_surface — `batched:` / `nondeterministic_columns` appear in docs-site
#    only in a retirement paragraph that also names the replacement (merge_key: /
#    columns.<c>.contract).
docs_site_no_retired_surface_check() {
  local bad=0
  while IFS=: read -r file line rest; do
    [[ -z "$file" ]] && continue
    # skip technique-name usages like "batched"/"keyed"/"versioned" maintenance techniques
    if [[ "$rest" == *'`batched`/`keyed`/`versioned`'* ]]; then
      continue
    fi
    # look at a small window (the mention plus the next 3 lines, e.g. an admonition body)
    # for the named replacement, not just the single matched line.
    local window
    window=$(sed -n "${line},$((line + 3))p" "$file")
    if ! grep -qE 'merge_key|columns\.<c>\.contract|safety_overrides' <<<"$window"; then
      fail "docs_site_no_retired_surface: $file:$line mentions retired surface with no replacement named nearby: $rest"
      bad=1
    fi
  done < <(rg -n 'batched:|nondeterministic_columns' docs-site/docs/ -g '*.md' 2>/dev/null || true)
  [[ $bad -eq 0 ]] && pass "docs_site_no_retired_surface"
}
docs_site_no_retired_surface_check

# 6. prior_phase_checks — phases 02-09 all still pass (06 is red today on IP-01/IP-02/MP-33
#    until this phase reclassifies them).
prior_phase_checks_check() {
  local bad=0
  for n in 02 03 04 05 06 07 08 09; do
    local script="docs/outcomes/20260809-incremental-spec-redraft/phases/${n}-check.sh"
    [[ -f "$script" ]] || continue
    if ! bash "$script" >/tmp/phase10_prior_${n}.log 2>&1; then
      fail "prior_phase_checks: phases/${n}-check.sh failed (see /tmp/phase10_prior_${n}.log)"
      bad=1
    fi
  done
  [[ $bad -eq 0 ]] && pass "prior_phase_checks"
}
prior_phase_checks_check

echo
if [[ "$FAIL" -eq 0 ]]; then
  echo "ALL PASS"
  exit 0
else
  echo "SOME CHECKS FAILED"
  exit 1
fi
