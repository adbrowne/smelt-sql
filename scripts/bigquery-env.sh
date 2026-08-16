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
