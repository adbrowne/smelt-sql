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
        version="${SMELT_PYPI_VERSION:-smelt-sql>=0.3.2}"
        echo "installing from PyPI: $version" >&2
        uv pip install --python "$PY" --refresh "$version" >&2
        ;;
    *)
        echo "unknown mode: $mode (expected local|pypi)" >&2
        exit 1
        ;;
esac

uv pip install --python "$PY" duckdb >&2

# Smoke checks
"$run_dir/project/.venv/bin/smelt" --help > /dev/null
# Verify docs list produces at least one topic (catches 0.3.1 which exits 0 but prints an error)
topic_count="$("$run_dir/project/.venv/bin/smelt" docs list 2>/dev/null | wc -l)"
if [[ "$topic_count" -lt 1 ]]; then
    smelt_ver="$("$run_dir/project/.venv/bin/smelt" --version 2>/dev/null || echo unknown)"
    echo "smoke check failed: 'smelt docs list' produced no output (smelt version: $smelt_ver)" >&2
    echo "pypi mode requires smelt-sql >= 0.3.2; install a newer release or use --mode local" >&2
    exit 1
fi

# Update a `latest` symlink under the parent runs dir so the most recent
# build is easy to find later (e.g. `cd ~/.smelt-test-runs/latest/project`).
parent_dir="$(dirname "$run_dir")"
ln -sfn "$run_dir" "$parent_dir/latest"

# Tell the caller where the agent's project code will land — these files
# persist after the loop finishes, so you can inspect what got built.
echo "agent project code:  $run_dir/project" >&2
echo "latest run symlink:  $parent_dir/latest" >&2

# Print the run_dir on stdout so callers can pipe / capture
echo "$run_dir"
