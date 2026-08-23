#!/usr/bin/env bash
#
# Standard per-phase verification gate, bundled into ONE tool call.
#
# Runs the four checks every phase must pass (fmt, clippy, workspace tests,
# example diagnostics) and prints ONLY failures plus a one-line PASS/FAIL
# summary per gate. Full output of a failing gate is truncated to its tail —
# the failure context is at the end for cargo tooling.
#
# Rationale (token efficiency): the autonomy loop / /smelt:implement used to
# run these as 4-6 separate Bash calls whose full output entered the agent's
# context and was re-read on every subsequent turn. One call with
# pre-truncated output keeps the transcript small.
#
# Usage:
#   bash .claude/scripts/verify-phase.sh            # all four gates
#   bash .claude/scripts/verify-phase.sh --fast     # skip the full `cargo test`
#                                                   # (fmt + clippy + example_diagnostics)
#
# Exit code: 0 = all gates green; 1 = at least one gate failed.

set -u

FAST=0
[ "${1:-}" = "--fast" ] && FAST=1

TAIL_LINES="${TAIL_LINES:-40}"
overall=0

run_gate() {
  local name="$1"; shift
  local out
  out="$("$@" 2>&1)"
  local rc=$?
  if [ "$rc" -eq 0 ]; then
    echo "PASS  ${name}"
  else
    overall=1
    echo "FAIL  ${name} (exit ${rc}) — last ${TAIL_LINES} lines (long lines truncated):"
    printf '%s\n' "$out" | tail -n "${TAIL_LINES}" | cut -c1-400 | sed 's/^/      /'
  fi
}

run_gate "cargo fmt --check"            cargo fmt --all -- --check
run_gate "cargo clippy (zero warnings, both feature sets)" bash .claude/scripts/clippy-gate.sh
if [ "$FAST" -eq 0 ]; then
  run_gate "cargo test (workspace)"     cargo test --quiet
fi
run_gate "example_diagnostics"          cargo test -p smelt-cli --test example_diagnostics --quiet

if [ "$overall" -eq 0 ]; then
  echo "VERIFY: ALL GREEN$([ "$FAST" -eq 1 ] && echo ' (fast mode — full cargo test SKIPPED)')"
else
  echo "VERIFY: FAILED — fix the gates above before marking the phase done"
fi
exit "$overall"
