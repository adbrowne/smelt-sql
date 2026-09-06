#!/usr/bin/env bash
# Large-file ratchet: freeze per-file .rs line counts to keep individual
# source files from growing into token-cost hot spots.
#
# Motivation: docs/research/20260906-outcome-hygiene-token-usage.md. Every
# `Edit` tool call on a large file echoes the whole file back into the
# calling session's context; a handful of oversized files being repeatedly
# edited by an autonomous loop was found to dominate its token spend.
#
# COUNTING RULE:
#   Every `*.rs` file under crates/*/src/ and crates/*/tests/ (production
#   code and test code both — the token-cost problem this guards against
#   applies to test files exactly as much as production ones). Excludes
#   anything under target/ (build output, never source).
#
# BASELINE SEMANTICS — deliberately ONE-SIDED, unlike this repo's other
# ratchets (hardening-budget.sh, parser-gaps-baseline.txt), which enforce an
# exact match and fail a "stale baseline" (current < baseline) the same as a
# regression. Line count is a continuously-varying metric that moves on
# almost any edit to a tracked file, unlike a discrete count (unwrap
# occurrences, registered parser gaps) that only changes via a deliberate
# action. Forcing `--update` on every incidental one-line shrink to a
# tracked file would fail CI on commits wholly unrelated to this gate's
# purpose. So:
#   - a tracked file growing past its baseline entry is a REGRESSION (fail)
#   - a tracked file shrinking below its baseline entry is fine, silently
#     (no forced --update; the baseline still records the historical high-
#     water mark until someone chooses to tighten it)
#   - a file with NO baseline entry is checked against DEFAULT_CAP_LINES
#     instead — this is what stops a new mega-file from being created
#     unchecked, or an existing untracked file from growing past the cap
#   - a baseline entry whose file no longer exists (deleted/renamed) is
#     still flagged as an ERROR — unlike the shrink case, a vanished file
#     is a distinct, deliberate event (rm/mv) worth requiring `--update` for,
#     not a routine byproduct of normal editing
#
# Usage:
#   large-file-check.sh           compare tree to baseline; exit 0 if OK
#   large-file-check.sh --update  write current line counts as new baseline
#
# Environment:
#   REPO_ROOT  override the repository root (used by tests to scan a fake tree)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${REPO_ROOT:-$(cd "$SCRIPT_DIR/../.." && pwd)}"
BASELINE_FILE="$REPO_ROOT/.claude/large-file-baseline.txt"
DEFAULT_CAP_LINES=1500
# Files below this size are never worth tracking individually — freezing a
# 20-line file's count is pure ratchet noise (see the incident that prompted
# this constant: nearly half the baseline was files under 200 lines, so
# routine edits to small files were failing as "regressions"). Only files at
# or above this line count get a baseline entry at all; everything smaller
# is left unconstrained until it grows past DEFAULT_CAP_LINES, at which
# point it becomes a "new oversized file" and earns a baseline entry.
BASELINE_MIN_LINES=1000
UPDATE_MODE=false
[[ "${1:-}" == "--update" ]] && UPDATE_MODE=true

# Collect current line counts, keyed by path relative to REPO_ROOT.
declare -A CURRENT
while IFS= read -r -d '' file; do
    rel="${file#"$REPO_ROOT"/}"
    lines="$(wc -l < "$file" | tr -d ' ')"
    CURRENT["$rel"]="$lines"
done < <(find "$REPO_ROOT"/crates -type d -name target -prune -o \
               \( -path '*/src/*.rs' -o -path '*/tests/*.rs' \) -type f -print0)

# --update: write current tree as the new baseline
if $UPDATE_MODE; then
    {
        echo "# Large-file ratchet baseline"
        echo "# Updated: $(date +%Y-%m-%d)"
        echo "# Format: <path relative to repo root> <line count>"
        echo "# One-sided: a tracked file may shrink freely; it may not grow"
        echo "# past this number without a reviewer sign-off note. A file with"
        echo "# no entry here is capped at ${DEFAULT_CAP_LINES} lines instead."
        echo "# Only files >= ${BASELINE_MIN_LINES} lines at update time get an entry —"
        echo "# smaller files are left unconstrained (see BASELINE_MIN_LINES in the script)."
        echo "# Regenerate with: .claude/scripts/large-file-check.sh --update"
        echo "#"
        for key in $(printf '%s\n' "${!CURRENT[@]}" | sort); do
            [[ "${CURRENT[$key]}" -lt "$BASELINE_MIN_LINES" ]] && continue
            echo "$key ${CURRENT[$key]}"
        done
    } > "$BASELINE_FILE"
    echo "Baseline written to $BASELINE_FILE"
    exit 0
fi

[[ -f "$BASELINE_FILE" ]] || {
    echo "ERROR: baseline file not found: $BASELINE_FILE"
    echo "Run '.claude/scripts/large-file-check.sh --update' to create it."
    exit 1
}

declare -A BASELINE
while IFS= read -r line; do
    [[ "$line" == \#* ]] && continue
    [[ -z "$line" ]] && continue
    read -r path count <<< "$line"
    BASELINE["$path"]="$count"
done < "$BASELINE_FILE"

exit_code=0

for key in $(printf '%s\n' "${!CURRENT[@]}" | sort); do
    current="${CURRENT[$key]}"

    if [[ -n "${BASELINE[$key]+set}" ]]; then
        baseline="${BASELINE[$key]}"
        if [[ "$current" -gt "$baseline" ]]; then
            echo "REGRESSION: $key: current=$current lines > baseline=$baseline lines"
            echo "  Split the file, revert the growth, or raise the baseline with a"
            echo "  reviewer sign-off note (.claude/scripts/large-file-check.sh --update)."
            exit_code=1
        fi
        # current < baseline: fine, no action required (see header rationale).
    else
        if [[ "$current" -gt "$DEFAULT_CAP_LINES" ]]; then
            echo "NEW OVERSIZED FILE: $key: $current lines > default cap of $DEFAULT_CAP_LINES"
            echo "  Split the file into cohesive submodules, or if the size is"
            echo "  justified, register it explicitly with"
            echo "  .claude/scripts/large-file-check.sh --update (reviewer sign-off note)."
            exit_code=1
        fi
    fi
done

# A baseline entry whose file no longer exists is a distinct, deliberate
# event (the file was deleted, renamed, or moved out of the scanned tree) —
# always worth a conscious --update, unlike an ordinary shrink.
for key in $(printf '%s\n' "${!BASELINE[@]}" | sort); do
    [[ -n "${CURRENT[$key]+set}" ]] && continue
    echo "ORPHANED BASELINE ENTRY: '$key' is registered but no longer exists."
    echo "  Run '.claude/scripts/large-file-check.sh --update' to drop the stale entry."
    exit_code=1
done

if [[ "$exit_code" -eq 0 ]]; then
    echo "Large-file ratchet OK — no tracked file exceeds its baseline, no new file exceeds ${DEFAULT_CAP_LINES} lines."
fi

exit "$exit_code"
