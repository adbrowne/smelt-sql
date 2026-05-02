#!/usr/bin/env bash
# Run the fixture's validate.py against a finished run directory.
# Writes <run_dir>/eval.json and exits 0 iff all checks pass.
#
# Usage: eval.sh <run_dir>
set -euo pipefail

run_dir="${1:?usage: eval.sh <run_dir>}"
[[ -d "$run_dir/project" ]] || { echo "no project dir at $run_dir/project" >&2; exit 2; }
[[ -f "$run_dir/.tier" ]] || { echo "no .tier marker; was setup_run.sh run?" >&2; exit 2; }

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$script_dir/../../.." && pwd)"
tier="$(cat "$run_dir/.tier")"
fixture="$repo/tests/agent-loop/fixtures/$tier"
PY="$run_dir/project/.venv/bin/python"

[[ -x "$PY" ]] || { echo "no venv python at $PY" >&2; exit 2; }

SMELT_PROJECT_DIR="$run_dir/project" \
SMELT_EVAL_OUT="$run_dir/eval.json" \
    "$PY" "$fixture/validate.py"
