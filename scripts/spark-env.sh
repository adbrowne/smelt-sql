# spark-env.sh — source this (`source scripts/spark-env.sh`) to point smelt's
# Spark integration tests at the local Spark Connect server started by
# scripts/spark-up.sh.
#
#   SPARK_CONNECT_URL     — gate + connect URL for the Spark backend tests.
#                           When UNSET, all Spark-targeted tests skip (green).
#   SMELT_SPARK_WAREHOUSE — host path Spark writes managed tables to and that
#                           host-side DuckDB reads for cross-engine Parquet.
#   PYSPARK_PYTHON / PYTHONPATH — the pinned client venv the PyO3 adapter imports
#                           pyspark from (must match the server's Spark version).
#
# The PyO3-embedded Python interpreter resolves `import pyspark` via PYTHONPATH,
# so we put the venv's site-packages on it rather than requiring an activated
# venv. Adjust SMELT_SPARK_PORT if you started the server on a non-default port.
_repo_root="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
export SPARK_CONNECT_URL="sc://localhost:${SMELT_SPARK_PORT:-15002}"
export SMELT_SPARK_WAREHOUSE="${SMELT_SPARK_WAREHOUSE:-${_repo_root}/.smelt-spark-warehouse}"

# The `smelt.spark_adapter` Python package lives in the repo's python/ dir; the
# pyspark client + its deps live in the pinned venv. Both must be importable by
# the PyO3-embedded interpreter, so put both on PYTHONPATH.
_venv_site="${_repo_root}/.smelt-spark-venv/lib/python3.12/site-packages"
export PYTHONPATH="${_repo_root}/python${PYTHONPATH:+:${PYTHONPATH}}"
if [ -d "${_venv_site}" ]; then
  export PYTHONPATH="${_venv_site}:${PYTHONPATH}"
  export PYSPARK_PYTHON="${_repo_root}/.smelt-spark-venv/bin/python"
fi

echo "SPARK_CONNECT_URL=${SPARK_CONNECT_URL}"
echo "SMELT_SPARK_WAREHOUSE=${SMELT_SPARK_WAREHOUSE}"
