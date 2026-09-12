#!/usr/bin/env bash
# dbx-dogfood-loader.sh — thin wrapper around dbx-dogfood-loader.py, sourcing
# the dogfood environment first so the pinned databricks-connect venv and the
# repo's python/ package are importable
# (docs/outcomes/20260912-databricks-dogfood-spine/phases/03-plan.md).
#
#     scripts/dbx-dogfood-loader.sh --emit-ddl
#     scripts/dbx-dogfood-loader.sh --emit-sql --date 2026-08-06
#     scripts/dbx-dogfood-loader.sh --emit-slice-sql --date 2026-08-06
#     scripts/dbx-dogfood-loader.sh --date 2026-08-06                # execute (needs a workspace)
#
# This is the only entry point criterion 4's `scripts/dbx-*.sh` allow-list
# needs for loading — the two `--emit-*` modes touch no network and need no
# credential, so a session can run them with nothing configured.
set -euo pipefail

_dbx_loader_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"

# shellcheck source=scripts/dbx-dogfood-env.sh
. "${_dbx_loader_dir}/dbx-dogfood-env.sh" >&2

_dbx_python="${_dbx_loader_dir}/../.smelt-dbx-venv/bin/python"
if [ ! -x "${_dbx_python}" ]; then
  _dbx_python="python3"
fi

exec "${_dbx_python}" "${_dbx_loader_dir}/dbx-dogfood-loader.py" "$@"
