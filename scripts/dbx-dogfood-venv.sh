#!/usr/bin/env bash
# dbx-dogfood-venv.sh — create (or refresh) the pinned `databricks-connect`
# client venv the Databricks backend's PyO3 adapter imports
# (docs/outcomes/20260912-databricks-dogfood-spine/phases/03-plan.md).
#
#     bash scripts/dbx-dogfood-venv.sh
#
# Mirrors scripts/bigquery-venv.sh and scripts/spark-up.sh's own venv setup,
# but MUST target its own directory, disjoint from the local-Spark
# integration tests' venv (scripts/spark-env.sh): `databricks-connect`
# conflicts with plain `pyspark` (both provide `pyspark.sql`).
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."

VENV=".smelt-dbx-venv"
PYVER="3.12"
REQUIREMENTS="scripts/dbx-dogfood-requirements.txt"

command -v uv >/dev/null 2>&1 || {
  echo "uv not found on PATH." >&2
  echo "Install it: curl -LsSf https://astral.sh/uv/install.sh | sh" >&2
  exit 1
}

echo "=== Creating ${VENV} (Python ${PYVER})"
uv venv --python "$PYVER" --allow-existing "$VENV"

echo
echo "=== Installing the Databricks Connect client"
uv pip install --python "${VENV}/bin/python" -r "$REQUIREMENTS"

echo
echo "=== Verifying the adapter imports"
if PYTHONPATH="$(pwd)/python" "${VENV}/bin/python" -c \
  'from smelt.databricks_adapter import DatabricksAdapter; print("smelt.databricks_adapter OK")'; then
  :
else
  echo "smelt.databricks_adapter failed to import against the pinned databricks-connect client" >&2
  exit 1
fi

echo
echo "Done — next: bash scripts/dbx-dogfood-loader.sh --emit-ddl"
