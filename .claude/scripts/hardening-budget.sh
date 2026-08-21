#!/usr/bin/env bash
# Hardening budget gate: freeze unwrap/expect/println debt in production code.
#
# COUNTING RULE (mandatory, not best-effort):
#   Patterns counted: .unwrap()   .expect("   println!
#   Production code means:
#     - Excludes files named tests.rs
#     - Excludes files under any tests/ directory
#     - Excludes lines from the first "#[cfg(test)]" line to end of file
#       (line-boundary approximation: a file with interleaved prod/test
#       sections may over-count slightly — the bias is conservative, i.e. it
#       produces false-alarm exits rather than false-pass exits)
#     - .expect(" matches the string-literal form to avoid false-positives
#       on the parser's self.expect(TOKEN) token-expectation method
#
# Usage:
#   hardening-budget.sh           compare tree to baseline; exit 0 if exact match
#   hardening-budget.sh --update  write current counts to baseline; always exit 0
#
# Environment:
#   REPO_ROOT  override the repository root (used by tests to scan a fake tree)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${REPO_ROOT:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
BASELINE_FILE="$REPO_ROOT/.claude/hardening-baseline.txt"
UPDATE_MODE=false
[[ "${1:-}" == "--update" ]] && UPDATE_MODE=true

# Count occurrences of a fixed-string pattern in the production portion of one
# .rs file (everything before the first "#[cfg(test)]" line).
# Uses awk for the fixed-string count so it always exits 0 and returns 0 for
# no matches (grep -c would exit 1 on 0 matches, causing set -e / pipefail
# issues and a spurious "|| echo 0" double-output bug).
_count_file() {
    local file="$1"
    local pattern="$2"
    sed -n '/^#\[cfg(test)\]/q;p' "$file" \
        | awk -v p="$pattern" '{n += (index($0, p) > 0)} END {print n+0}'
}

# Count pattern across all production .rs files under a src/ directory.
_count_crate() {
    local src_dir="$1"
    local pattern="$2"
    local total=0
    local n
    while IFS= read -r -d '' file; do
        local fname="${file##*/}"
        # Skip files named tests.rs
        [[ "$fname" == "tests.rs" ]] && continue
        # Skip files under tests/ subdirectories
        [[ "$file" == */tests/* ]] && continue
        n="$(_count_file "$file" "$pattern")"
        total=$((total + n))
    done < <(find "$src_dir" -name '*.rs' -print0 2>/dev/null)
    echo "$total"
}

# Print the dependency names listed under one manifest section, one per line.
# `kind` is "dependencies" or "dev-dependencies". Handles both the inline table
# form (`foo = { path = ... }`, `foo.workspace = true`) and the sub-table form
# (`[dependencies.foo]`). Section tracking is what keeps `[features]` entries
# like `smelt-maintenance-testkit/spark` from being mistaken for dependencies.
_deps_of_kind() {
    local manifest="$1"
    local kind="$2"
    awk -v want="$kind" '
        /^[ \t]*\[/ {
            s = $0; sub(/#.*/, "", s); gsub(/[ \t]/, "", s)
            section = s
            # [dependencies.foo] — the dependency name is the table suffix.
            if (index(section, "[" want ".") == 1) {
                name = substr(section, length(want) + 3)
                sub(/\]$/, "", name)
                print name
            }
            next
        }
        section == "[" want "]" {
            line = $0; sub(/#.*/, "", line)
            if (line ~ /^[ \t]*[A-Za-z0-9_-]+[ \t]*(\.[A-Za-z0-9_-]+)?[ \t]*=/) {
                name = line
                sub(/[ \t]*=.*/, "", name)   # drop everything from the "="
                sub(/\..*/, "", name)        # drop a `.workspace` suffix
                gsub(/[ \t]/, "", name)
                print name
            }
        }
    ' "$manifest" 2>/dev/null
}

# Test-support crates are DERIVED, never declared. A crate is test-support iff
# all three hold:
#
#   1. some crate names it under [dev-dependencies],
#   2. no crate names it under [dependencies], and
#   3. it has no binary target (no src/main.rs, no [[bin]] table).
#
# Its unwrap/expect debt is then excluded from the budget, because that debt is
# test scaffolding rather than production risk.
#
# Each condition is load-bearing, and (3) was learned the hard way. "Nothing
# depends on it" alone would excuse every top-level crate, which nothing depends
# on either. Requiring (1) is not enough on its own: `smelt-runtime` carries a
# test-only back edge onto `smelt-cli`, so the shipped CLI is dev-depended-upon
# and normally-depended-upon by nobody, and conditions (1)+(2) alone dropped it
# — and its 161 printlns — straight out of the budget. A crate that produces a
# binary ships to users and is production whatever the dependency edges say.
#
# The default is production: a crate escapes the budget only by demonstrating it
# is test-only, so an unparseable or absent manifest keeps its debt counted.
#
# Deriving this rather than keeping an exclusion list means promoting a testkit
# to a real dependency silently re-enters its debt into the gate.
declare -A IS_PROD_DEP=()
declare -A IS_DEV_DEP=()
for manifest in "$REPO_ROOT"/crates/*/Cargo.toml; do
    [[ -f "$manifest" ]] || continue
    while IFS= read -r dep; do
        [[ -n "$dep" ]] && IS_PROD_DEP["$dep"]=1
    done < <(_deps_of_kind "$manifest" "dependencies")
    while IFS= read -r dep; do
        [[ -n "$dep" ]] && IS_DEV_DEP["$dep"]=1
    done < <(_deps_of_kind "$manifest" "dev-dependencies")
done

_has_binary_target() {
    local crate_dir="$1"
    [[ -f "$crate_dir/src/main.rs" ]] && return 0
    grep -q '^[ \t]*\[\[bin\]\]' "$crate_dir/Cargo.toml" 2>/dev/null
}

_is_test_support_crate() {
    local crate="$1"
    local crate_dir="$2"
    [[ -n "${IS_DEV_DEP[$crate]+set}" ]] || return 1
    [[ -z "${IS_PROD_DEP[$crate]+set}" ]] || return 1
    ! _has_binary_target "$crate_dir"
}

# Collect current counts for all crates
declare -A CURRENT
expect_pat='.expect("'
for crate_src_dir in "$REPO_ROOT"/crates/*/src; do
    [[ -d "$crate_src_dir" ]] || continue
    crate_dir="$(dirname "$crate_src_dir")"
    crate="$(basename "$crate_dir")"
    if _is_test_support_crate "$crate" "$crate_dir"; then
        continue
    fi
    CURRENT["$crate:unwrap"]="$(_count_crate "$crate_src_dir" ".unwrap()")"
    CURRENT["$crate:expect"]="$(_count_crate "$crate_src_dir" "$expect_pat")"
    CURRENT["$crate:println"]="$(_count_crate "$crate_src_dir" "println!")"
done

# --update: write current tree as the new baseline
if $UPDATE_MODE; then
    {
        echo "# Production code debt baseline"
        echo "# Updated: $(date +%Y-%m-%d)"
        echo "# Format: <crate> <pattern> <count>"
        echo "# Patterns: unwrap  expect  println"
        echo "# Regenerate with: .claude/scripts/hardening-budget.sh --update"
        echo "#"
        for key in $(printf '%s\n' "${!CURRENT[@]}" | sort); do
            crate="${key%%:*}"
            pattern="${key##*:}"
            echo "$crate $pattern ${CURRENT[$key]}"
        done
    } > "$BASELINE_FILE"
    echo "Baseline written to $BASELINE_FILE"
    exit 0
fi

# Read baseline
[[ -f "$BASELINE_FILE" ]] || {
    echo "ERROR: baseline file not found: $BASELINE_FILE"
    echo "Run '.claude/scripts/hardening-budget.sh --update' to create it."
    exit 1
}

declare -A BASELINE
while IFS= read -r line; do
    # Skip comments and blank lines
    [[ "$line" == \#* ]] && continue
    [[ -z "$line" ]] && continue
    read -r crate pattern count <<< "$line"
    BASELINE["$crate:$pattern"]="$count"
done < "$BASELINE_FILE"

# Compare
exit_code=0

# Every crate in the current tree must be registered in the baseline
for key in $(printf '%s\n' "${!CURRENT[@]}" | sort); do
    crate="${key%%:*}"
    pattern="${key##*:}"
    current="${CURRENT[$key]}"

    if [[ -z "${BASELINE[$key]+set}" ]]; then
        echo "ERROR: unregistered crate/pattern '$crate $pattern' not in baseline."
        echo "  Run '.claude/scripts/hardening-budget.sh --update' to register it."
        exit_code=1
        continue
    fi

    baseline="${BASELINE[$key]}"

    if [[ "$current" -gt "$baseline" ]]; then
        echo "REGRESSION: $crate $pattern: current=$current > baseline=$baseline"
        echo "  Revert the regression or justify it by updating the baseline."
        exit_code=1
    elif [[ "$current" -lt "$baseline" ]]; then
        echo "STALE BASELINE: $crate $pattern: current=$current < baseline=$baseline"
        echo "  Run '.claude/scripts/hardening-budget.sh --update' to tighten the baseline."
        exit_code=1
    fi
done

# ...and every baseline entry must still correspond to a counted crate. Without
# this direction the ratchet is one-sided: a crate that silently drops out of
# the budget — deleted, renamed, or newly derived as test-support — leaves its
# entry sitting in the baseline, unchecked and indistinguishable from a crate
# still being measured. That is the same class of invisible drift the
# test-support derivation exists to prevent, so it is checked here rather than
# left to whoever next runs --update.
for key in $(printf '%s\n' "${!BASELINE[@]}" | sort); do
    [[ -n "${CURRENT[$key]+set}" ]] && continue
    crate="${key%%:*}"
    pattern="${key##*:}"
    echo "ORPHANED BASELINE ENTRY: '$crate $pattern' is registered but no longer counted."
    echo "  The crate was removed, renamed, or is now derived as test-support"
    echo "  (dev-dependency of some crate, regular dependency of none, no binary target)."
    echo "  Run '.claude/scripts/hardening-budget.sh --update' to drop the stale entry."
    exit_code=1
done

if [[ "$exit_code" -eq 0 ]]; then
    echo "Hardening budget OK — all production counts match baseline."
fi

exit "$exit_code"
