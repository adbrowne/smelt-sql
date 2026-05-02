#!/usr/bin/env bash
# Prepare a fresh run directory for a smelt build agent:
#   - skill.md and spec.md copied in
#   - seeds CSVs from the fixture copied to project/seeds/
#   - .venv with smelt installed (local wheel or PyPI)
#   - smoke check: smelt --help and smelt docs list both work
#
# Usage: setup_run.sh <tier> <mode> <run_dir>
#   tier:    small | medium | large  (must match a fixture dir)
#   mode:    local | pypi
#   run_dir: absolute path; will be created if missing, must be empty
set -euo pipefail

tier="${1:?usage: setup_run.sh <tier> <mode> <run_dir>}"
mode="${2:?usage: setup_run.sh <tier> <mode> <run_dir>}"
run_dir="${3:?usage: setup_run.sh <tier> <mode> <run_dir>}"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$script_dir/../../.." && pwd)"
fixture="$repo/tests/agent-loop/fixtures/$tier"
harness="$repo/tests/agent-loop/harness"

[[ -d "$fixture" ]] || { echo "no fixture at $fixture" >&2; exit 1; }

if [[ -e "$run_dir" ]] && [[ -n "$(ls -A "$run_dir" 2>/dev/null)" ]]; then
    echo "run_dir $run_dir is not empty; refusing to overwrite" >&2
    exit 1
fi

mkdir -p "$run_dir/project/seeds" "$run_dir/artifacts"

# Persist tier so eval.sh can find the fixture later
echo "$tier" > "$run_dir/.tier"
echo "$mode" > "$run_dir/.mode"

cp "$repo/.claude/skills/smelt-app-builder/SKILL.md" "$run_dir/skill.md"
cp "$fixture/spec.md" "$run_dir/spec.md"
cp "$fixture/seeds/"*.csv "$run_dir/project/seeds/"

cd "$run_dir/project"
uv venv --python 3.11 >&2
PY="$run_dir/project/.venv/bin/python"

case "$mode" in
    local)
        wheel="$(bash "$harness/build_local_wheel.sh")"
        echo "installing local wheel: $wheel" >&2
        uv pip install --python "$PY" "$wheel" >&2
        ;;
    pypi)
        # Published 0.3.1 doesn't ship `smelt docs`, so the agent would have
        # no way to read docs in pypi mode. Re-enable once a release with
        # embedded docs (>= 0.3.2) is published.
        echo "pypi mode is disabled until a release with embedded docs ships;" >&2
        echo "use --mode local for now." >&2
        exit 1
        ;;
    *)
        echo "unknown mode: $mode (expected local|pypi)" >&2
        exit 1
        ;;
esac

uv pip install --python "$PY" duckdb >&2

# Smoke checks
"$run_dir/project/.venv/bin/smelt" --help > /dev/null
"$run_dir/project/.venv/bin/smelt" docs list > /dev/null

# Print the run_dir so the caller can pipe / capture
echo "$run_dir"
