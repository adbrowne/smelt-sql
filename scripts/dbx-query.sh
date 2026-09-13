#!/usr/bin/env bash
# dbx-query.sh — run one read-only SQL statement against the Databricks
# dogfood workspace and print its rows as newline-delimited JSON.
#
#     bash scripts/dbx-query.sh "SELECT count(*) FROM workspace.smelt_dogfood.github_events"
#
# The only allow-listed way a Claude session touches the workspace directly
# (docs/outcomes/20260912-databricks-dogfood-spine/phases/04a-plan.md) — it
# never touches the encrypted secret, only the already-minted token that
# scripts/dbx-auth.sh produced.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"

# shellcheck source=scripts/dbx-dogfood-env.sh
. "${REPO_ROOT}/scripts/dbx-dogfood-env.sh" >&2

_dbx_python="${REPO_ROOT}/.smelt-dbx-venv/bin/python"
if [ ! -x "${_dbx_python}" ]; then
  _dbx_python="python3"
fi

exec "${_dbx_python}" "${REPO_ROOT}/scripts/dbx_dogfood_query.py" "${1:?usage: dbx-query.sh <sql>}"
