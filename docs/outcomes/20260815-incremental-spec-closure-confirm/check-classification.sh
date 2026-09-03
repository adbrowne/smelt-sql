#!/usr/bin/env bash
# Verifies every baseline bullet in baseline-inventory.md has a real Disposition
# (not TBD, first word one of closed|open|drifted|residue), that every `closed`
# row cites a commit that actually exists, and that `closed`/`residue` rows are
# absent from the current (HEAD) Known Divergences text while `open` rows are
# still present there — via current-inventory.tsv, generated with
# `bash extract-baseline.sh HEAD`. `drifted` rows are exempted from the
# presence/absence check by definition (their whole point is that the wording
# changed relative to baseline — an exact-lead-in match would either falsely
# fail a reworded-but-still-present row or falsely pass a moved-to-Future-
# Extensions row); phase 3 owns the judgement call on whether a `drifted` row's
# *current* wording is itself stale.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INVENTORY_MD="$DIR/baseline-inventory.md"
CURRENT_TSV="$DIR/current-inventory.tsv"

if [ ! -f "$CURRENT_TSV" ]; then
  echo "FAIL: $CURRENT_TSV missing — regenerate with: bash $DIR/extract-baseline.sh HEAD > $CURRENT_TSV" >&2
  exit 1
fi

declare -A prefix_spec=( [DD]=definition_deltas [IM]=incremental_models [IS]=incremental_shapes [MP]=model_properties )

fail=0
offenders=()

# collapse whitespace the same way extract-baseline.sh does, for substring comparison
collapse() {
  printf '%s' "$1" | tr -s '[:space:]' ' ' | sed -e 's/^ //' -e 's/ $//'
}

trim() {
  printf '%s' "$1" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//'
}

while IFS= read -r row; do
  # protect an escaped "\|" (a literal pipe inside a table cell, e.g. IM-18's
  # `on_column_add: backfill \| leave_null \| recompute`) from the column split
  row="${row//\\|/$'\x01'}"
  IFS='|' read -r _ id _subsection bullet oq disposition _ <<<"$row"
  id="${id//$'\x01'/|}"
  bullet="${bullet//$'\x01'/|}"
  disposition="${disposition//$'\x01'/|}"
  id="$(trim "$id")"
  [[ "$id" =~ ^(DD|IM|IS|MP)-[0-9]+$ ]] || continue
  bullet="$(trim "$bullet")"
  disposition="$(trim "$disposition")"
  prefix="${id%%-*}"
  spec="${prefix_spec[$prefix]}"

  if [ -z "$disposition" ] || [[ "$disposition" == TBD* ]]; then
    echo "FAIL: $id has no disposition (still TBD)" >&2
    offenders+=("$id")
    fail=1
    continue
  fi

  first_word="${disposition%% *}"
  case "$first_word" in
    closed|open|drifted|residue) ;;
    *)
      echo "FAIL: $id disposition '$disposition' does not start with closed|open|drifted|residue" >&2
      offenders+=("$id")
      fail=1
      continue
      ;;
  esac

  # bold lead-in is the text before the trailing " (L<n>)" baseline-line-number suffix
  lead="$(echo "$bullet" | sed -E 's/ \(L[0-9]+\)\s*$//')"
  lead_collapsed="$(collapse "$lead")"

  if [ "$first_word" = "closed" ]; then
    sha="$(echo "$disposition" | awk '{print $2}' | sed 's/[^0-9a-f]*$//')"
    if [ -z "$sha" ] || ! git -C "$DIR" cat-file -e "$sha" 2>/dev/null; then
      echo "FAIL: $id disposition '$disposition' does not cite a resolvable commit" >&2
      offenders+=("$id")
      fail=1
    fi
  fi

  if [ "$first_word" = "closed" ] || [ "$first_word" = "residue" ]; then
    if grep -F -q -- "$lead_collapsed" <(awk -F'\t' -v s="$spec" '$1 == s {print $3}' "$CURRENT_TSV"); then
      echo "FAIL: $id classified '$first_word' but its lead-in is still present verbatim in the current Known Divergences section" >&2
      offenders+=("$id")
      fail=1
    fi
  fi

  if [ "$first_word" = "open" ]; then
    if ! grep -F -q -- "$lead_collapsed" <(awk -F'\t' -v s="$spec" '$1 == s {print $3}' "$CURRENT_TSV"); then
      echo "FAIL: $id classified 'open' but its lead-in is absent from the current Known Divergences section" >&2
      offenders+=("$id")
      fail=1
    fi
  fi
done < <(grep -E '^\| (DD|IM|IS|MP)-[0-9]+ \|' "$INVENTORY_MD")

if [ "$fail" -ne 0 ]; then
  echo "FAIL: offending IDs: ${offenders[*]}" >&2
  exit 1
fi

echo "OK: all 80 baseline bullets have a valid, repo-verified disposition"
