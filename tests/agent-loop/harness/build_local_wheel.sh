#!/usr/bin/env bash
# Build a smelt-sql wheel from the current worktree using maturin.
# Prints the absolute path of the resulting wheel on stdout. Re-runs are
# cheap because maturin/cargo cache aggressively.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$script_dir/../../.." && pwd)"
cd "$repo"
mkdir -p dist
# pyproject.toml's `data = "smelt_sql.data"` requires this dir to exist;
# CI populates it but locally it's gitignored, so make sure it's there.
mkdir -p smelt_sql.data/scripts
# Default to debug build (fast). Set SMELT_WHEEL_RELEASE=1 to build a release
# wheel (slower but more representative of what users install).
profile_flag=""
if [[ "${SMELT_WHEEL_RELEASE:-0}" == "1" ]]; then
    profile_flag="--release"
fi

# Pin the wheel's Python ABI tag to the interpreter the harness installs into
# (3.11 by default); without this, maturin tags the wheel for whatever
# interpreter its own tool venv uses (often newer than the target venv).
target_python="${SMELT_BUILD_PYTHON:-$(uv python find 3.11)}"

# --auditwheel skip avoids renaming/relocating libduckdb.so; we control the
# install env and know the system has libduckdb available.
uvx maturin build $profile_flag --interpreter "$target_python" --auditwheel skip --out dist/ >&2

# Emit absolute path so callers in any cwd can resolve it
ls -t "$repo/dist"/*.whl | head -1
