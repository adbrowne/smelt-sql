# bigquery-env.sh — source this (`source scripts/bigquery-env.sh`) to point
# smelt's BigQuery integration tests at your provisioned GCP project.
#
#   SMELT_BQ_PROJECT      — gate + target project for the BigQuery backend tests.
#                           When UNSET, all BigQuery-targeted tests skip (green).
#   SMELT_BQ_DATASET      — base dataset; per-run test datasets are suffixed.
#   SMELT_BQ_LOCATION     — dataset location (must match at query time).
#   SMELT_BQ_ACCESS_TOKEN — short-lived OAuth token from bigquery-auth.sh.
#
# The adapter authenticates with SMELT_BQ_ACCESS_TOKEN *explicitly* and never
# falls back to application-default credentials. That is deliberate: it makes
# ambient credentials unusable, so the only way to reach GCP is the token this
# script exports.
_bq_repo_root="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"

# The PyO3-embedded interpreter resolves `import smelt.bigquery_adapter` via
# PYTHONPATH, so both the repo's python/ package and the pinned client venv's
# site-packages must be importable without an activated venv — the same shape
# scripts/spark-env.sh uses.
export PYTHONPATH="${_bq_repo_root}/python${PYTHONPATH:+:${PYTHONPATH}}"
_bq_venv_site="${_bq_repo_root}/.smelt-bq-venv/lib/python3.12/site-packages"
if [ -d "${_bq_venv_site}" ]; then
  export PYTHONPATH="${_bq_venv_site}:${PYTHONPATH}"
else
  echo "no BigQuery client venv — create it with: bash scripts/bigquery-venv.sh" >&2
fi

# Every dataset the tests create inherits this default table expiration, so an
# interrupted run sheds its tables even if teardown never ran. Both the test
# process and the `smelt run` children it spawns read it from the environment.
export SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS="${SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS:-7200000}"

_bq_config_dir="${SMELT_BQ_CONFIG_DIR:-$HOME/.config/gcloud-smelt-bq}"
_bq_env_file="${_bq_config_dir}/config.env"

if [ -f "${_bq_env_file}" ]; then
  # shellcheck disable=SC1090
  . "${_bq_env_file}"
  export SMELT_BQ_PROJECT SMELT_BQ_DATASET SMELT_BQ_LOCATION
else
  echo "no config at ${_bq_env_file} — run: bash scripts/bigquery-key.sh <project-id>" >&2
fi

_bq_token_file="${_bq_config_dir}/token"
if [ -f "${_bq_token_file}" ]; then
  _bq_expires="$(sed -n '2p' "${_bq_token_file}")"
  if [ -n "${_bq_expires}" ] && [ "$(date +%s)" -lt "${_bq_expires}" ]; then
    SMELT_BQ_ACCESS_TOKEN="$(sed -n '1p' "${_bq_token_file}")"
    export SMELT_BQ_ACCESS_TOKEN
    echo "SMELT_BQ_ACCESS_TOKEN valid until $(date -d "@${_bq_expires}" '+%H:%M:%S')"
  else
    unset SMELT_BQ_ACCESS_TOKEN
    echo "BigQuery token expired — run: bash scripts/bigquery-auth.sh" >&2
  fi
else
  echo "no BigQuery token — run: bash scripts/bigquery-auth.sh" >&2
fi

echo "SMELT_BQ_PROJECT=${SMELT_BQ_PROJECT:-<unset>}"
echo "SMELT_BQ_DATASET=${SMELT_BQ_DATASET:-<unset>}"
