#!/usr/bin/env bash
# bigquery-venv.sh — create (or refresh) the pinned Python client venv that the
# BigQuery backend's PyO3 adapter imports.
#
#     bash scripts/bigquery-venv.sh
#
# The Rust backend bridges to python/smelt/bigquery_adapter.py, which needs
# google-cloud-bigquery and pyarrow. Those live in a repo-local venv rather than
# in the system interpreter, mirroring .smelt-spark-venv — scripts/bigquery-env.sh
# puts its site-packages on PYTHONPATH so the embedded interpreter resolves the
# import without an activated venv.
#
# uv creates the environment. It bundles its own Python, so this works on hosts
# whose system interpreter has no pip or ensurepip (Debian/Ubuntu split those
# into separate packages) — which is the common case here.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."

VENV=".smelt-bq-venv"
# Pinned to 3.12 because bigquery-env.sh puts a versioned site-packages path on
# PYTHONPATH; bumping this means bumping that path too.
PYVER="3.12"

command -v uv >/dev/null 2>&1 || {
  echo "uv not found on PATH." >&2
  echo "Install it: curl -LsSf https://astral.sh/uv/install.sh | sh" >&2
  exit 1
}

echo "=== Creating ${VENV} (Python ${PYVER})"
# --allow-existing keeps this re-runnable: refreshing the client on an existing
# venv is the common case, and re-creating it from scratch is not the caller's
# intent. Pass --clear yourself when you do want a rebuild.
uv venv --python "$PYVER" --allow-existing "$VENV"

echo
echo "=== Installing the BigQuery client"
uv pip install --python "${VENV}/bin/python" google-cloud-bigquery pyarrow

echo
echo "=== Verifying the adapter imports"
PYTHONPATH="$(pwd)/python" "${VENV}/bin/python" -c \
  'from smelt.bigquery_adapter import BigQueryAdapter; print("smelt.bigquery_adapter OK")'

echo
echo "Done — next: bash scripts/bigquery-auth.sh, then bash scripts/bigquery-test.sh"
