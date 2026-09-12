#!/usr/bin/env bash
#
# Shell-lint gate over the repo's own scripts (this file is
# .claude/scripts/shellcheck-gate.sh — the name cannot open a comment line
# here, since a comment starting with the tool's name parses as a directive).
#
# Scope: scripts/ and .claude/scripts/ — the provisioning, credential and loop
# drivers this repo actually maintains. Deliberately NOT a repo-wide sweep:
# docs/demos/node_modules/ carries ~50 vendored playwright scripts that are
# somebody else's code and would make the gate meaningless.
#
# Severity: `warning` and above. Both directories are clean at that level as of
# 2026-09-09, so this is a zero-findings gate with no ratchet file — a new
# finding fails immediately rather than being absorbed into a budget. The
# `info` level (10x SC2015 `A && B || C`, 2x SC2012 `ls` in a pipeline) is
# deliberately out of scope: those are idiom preferences here, not defects.
#
# A deliberate exception belongs inline, as a `# shellcheck disable=SCxxxx`
# with a reason on the same line — never by lowering the severity here.
#
# Usage:
#   bash .claude/scripts/shellcheck-gate.sh     # or: mise run shellcheck
#
# Exit code: 0 = clean; 1 = findings (printed); 2 = shellcheck missing.

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
cd "${REPO_ROOT}" || exit 1

SEVERITY="${SHELLCHECK_SEVERITY:-warning}"

# Resolve the binary rather than requiring the caller to be inside a
# mise-managed shell. CLAUDE.md documents `bash .claude/scripts/verify-phase.sh`
# as the gate's invocation, and a bare PATH carries no mise shims — so
# insisting on `mise exec --` would make the documented command fail for a
# reason with nothing to do with the code under test.
SHELLCHECK="$(command -v shellcheck 2>/dev/null || true)"
if [ -z "$SHELLCHECK" ] && command -v mise >/dev/null 2>&1; then
  SHELLCHECK="$(mise which shellcheck 2>/dev/null || true)"
fi
if [ -z "$SHELLCHECK" ]; then
  # Fail rather than skip. A silently-skipped lint gate is a hole that reads
  # exactly like a pass — the same failure mode as an unset DUCKDB_LIB_DIR.
  echo "shellcheck not found on PATH, and mise could not resolve it either." >&2
  echo "It is pinned in mise.toml's [tools] — run: mise install" >&2
  exit 2
fi

# Nothing is passed through a pipe to shellcheck: it takes the file list
# directly so its own exit status is the gate's.
mapfile -t targets < <(
  find scripts .claude/scripts -maxdepth 1 -name '*.sh' -type f | sort
)

if [ "${#targets[@]}" -eq 0 ]; then
  echo "no shell scripts found — check this gate's scope" >&2
  exit 1
fi

if "$SHELLCHECK" -S "${SEVERITY}" -f gcc "${targets[@]}"; then
  echo "PASS  shellcheck (${#targets[@]} scripts, severity ${SEVERITY}, zero findings)"
  exit 0
else
  echo "FAIL  shellcheck — fix the findings above, or add an inline"
  echo "      '# shellcheck disable=SCxxxx  # <reason>' where the code is right."
  exit 1
fi
