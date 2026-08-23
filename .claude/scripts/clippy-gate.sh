#!/usr/bin/env bash
#
# The clippy gate — SINGLE SOURCE OF TRUTH for both CI and the local
# pre-commit gate.
#
# Why this file exists: CI's lint job and `.claude/scripts/verify-phase.sh`
# used to spell the clippy invocation out separately, and they drifted. CI ran
# `--no-default-features --features smelt-cli/duckdb,smelt-ui/duckdb`; the local
# gate ran a plain `cargo clippy --all-targets`. A warning that only appears
# under the CI feature set therefore survived a green local run and failed in
# CI with no code change. Both callers now invoke this script, so the two gates
# cannot diverge again.
#
# Two feature sets are linted because neither subsumes the other:
#
#   1. default    — what `cargo test`/`cargo build` actually compile. For
#                   smelt-cli/smelt-ui `default = ["duckdb"]`, but every OTHER
#                   workspace crate also keeps its defaults here.
#   2. ci-minimal — `--no-default-features` strips defaults workspace-wide and
#                   re-adds only the duckdb backends. Different `cfg` surface,
#                   different lints.
#
# Usage:
#   bash .claude/scripts/clippy-gate.sh              # both feature sets
#   bash .claude/scripts/clippy-gate.sh default      # just the default set
#   bash .claude/scripts/clippy-gate.sh ci-minimal   # just the CI set
#
# Exit code: 0 = both (or the named) invocations clean; 1 = at least one failed.

set -u

WHICH="${1:-both}"
rc=0

run_default() {
  echo "== clippy (default features) =="
  cargo clippy --all-targets -- -D warnings || rc=1
}

run_ci_minimal() {
  echo "== clippy (--no-default-features + duckdb backends) =="
  cargo clippy \
    --all-targets \
    --no-default-features \
    --features smelt-cli/duckdb,smelt-ui/duckdb \
    -- -D warnings || rc=1
}

case "$WHICH" in
  default)    run_default ;;
  ci-minimal) run_ci_minimal ;;
  both)       run_default; run_ci_minimal ;;
  *)
    echo "usage: $0 [both|default|ci-minimal]" >&2
    exit 2
    ;;
esac

exit "$rc"
