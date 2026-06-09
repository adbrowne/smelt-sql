#!/usr/bin/env bash
# Unknown-census: list every DataType::Unknown CONSTRUCTION site in production code.
#
# WHAT COUNTS AS A CONSTRUCTION SITE:
#   A line that *produces* the DataType::Unknown value — struct field initialiser,
#   `unwrap_or`/`map_or` fallback, match arm body, function return, etc.
#
# EXCLUDED:
#   1. Test code (files named tests.rs, files under tests/ dirs, lines after
#      the first `#[cfg(test)]` in a file).  Same rule as hardening-budget.sh.
#   2. Pure comment lines (trimmed line starts with // or ///).
#   3. Match arm PATTERNS: lines where `DataType::Unknown` is the SCRUTINEE —
#      identified by position: if the first occurrence of `DataType::Unknown`
#      on a line appears BEFORE the first `=>` on the same line, it is a
#      pattern (excluded).  If `DataType::Unknown` appears AFTER the first `=>`,
#      or there is no `=>` on the line, it is a construction (included).
#      Covers:  `DataType::Unknown =>`,
#               `DataType::Unknown | DataType::Null =>`, etc.
#   4. Comparison / guard expressions:
#        matches!(... DataType::Unknown ...)
#        == DataType::Unknown
#        != DataType::Unknown
#
# OUTPUT FORMAT:
#   One line per site: <relative-path-from-repo-root>:<line-number>
#   Output is sorted for reproducibility.
#
# Usage:
#   .claude/scripts/unknown-census.sh
#   REPO_ROOT=/path/to/repo .claude/scripts/unknown-census.sh
#
# The test `crates/smelt-types/tests/unknown_census.rs` runs this script and
# asserts its output matches .claude/unknown-census.toml exactly.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${REPO_ROOT:-$(cd "$SCRIPT_DIR/../.." && pwd)}"

results=()

while IFS= read -r -d '' src_file; do
    fname="${src_file##*/}"
    rel="${src_file#"$REPO_ROOT/"}"

    # Skip test-named files
    [[ "$fname" == "tests.rs" ]] && continue

    # Skip files under tests/ directories
    [[ "$src_file" == */tests/* ]] && continue

    # Truncate at first #[cfg(test)] line
    prod_lines="$(sed -n '/^#\[cfg(test)\]/q;p' "$src_file")"

    # For each matching line: apply exclusions, emit if it is a construction site
    while IFS= read -r line; do
        lineno="${line%%:*}"
        content="${line#*:}"

        # --- Exclusion 1: pure comment line ---
        trimmed="${content#"${content%%[! ]*}"}"   # ltrim whitespace
        [[ "$trimmed" == "//"* ]] && continue

        # --- Exclusion 2: match arm PATTERN (DataType::Unknown is the scrutinee) ---
        # Heuristic: if the first byte-position of "DataType::Unknown" on this
        # line is LESS THAN the first byte-position of "=>", then DataType::Unknown
        # is in the pattern side of a match arm → exclude.
        du_pos=$(echo "$content" | grep -bo 'DataType::Unknown' 2>/dev/null | head -1 | cut -d: -f1 || true)
        arr_pos=$(echo "$content" | grep -bo '=>' 2>/dev/null | head -1 | cut -d: -f1 || true)
        if [[ -n "$du_pos" && -n "$arr_pos" ]] && (( du_pos < arr_pos )); then
            continue
        fi

        # --- Exclusion 3: matches! guard ---
        echo "$content" | grep -q 'matches!(.*DataType::Unknown' 2>/dev/null && continue || true

        # --- Exclusion 4: equality comparison ---
        echo "$content" | grep -q '==\s*DataType::Unknown' 2>/dev/null && continue || true
        echo "$content" | grep -q '!=\s*DataType::Unknown' 2>/dev/null && continue || true

        # Passed all exclusions — this is a production construction site
        results+=("$rel:$lineno")
    done < <(
        echo "$prod_lines" | grep -n 'DataType::Unknown'
    )

done < <(find "$REPO_ROOT/crates" -path '*/src/*.rs' -print0 | sort -z)

# Sort and print
printf '%s\n' "${results[@]}" | sort
