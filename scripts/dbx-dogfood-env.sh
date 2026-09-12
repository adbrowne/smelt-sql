# shellcheck shell=bash  # sourced, not executed — no shebang by design
# dbx-dogfood-env.sh — source this (`source scripts/dbx-dogfood-env.sh`) to
# point the github_activity dogfood pipeline at the Databricks Free Edition
# workspace (docs/outcomes/20260912-databricks-dogfood-spine/phases/03-plan.md).
#
#   SMELT_DBX_HOST     — gate + scheme-bearing workspace URL (e.g.
#                        `https://dbc-xxxx.cloud.databricks.com`), consumed
#                        directly by URL-building callers (`dbx-auth.sh`,
#                        the Python query wrapper). When UNSET, all
#                        Databricks-targeted tests skip (green).
#   SMELT_DBX_HOSTNAME — SMELT_DBX_HOST with its scheme and any trailing
#                        slash stripped — the bare-hostname shape the
#                        `type: databricks` target's `host:` key requires
#                        (`docs/specs/smelt_yml.md` §"Target shape"). This is
#                        the variable `examples/github_activity/smelt.yml`
#                        interpolates into `host:`, never SMELT_DBX_HOST
#                        itself.
#   SMELT_DBX_TOKEN    — a short-lived PAT/OAuth token, read from the
#                        gpg-backed config dir's decrypted `token` file if
#                        present. Never printed by this script. Unset (rather
#                        than empty) is the *ambient*-credential form the
#                        databricks target also supports
#                        (docs/specs/smelt_yml.md §"Target shape").
#   SMELT_DBX_CATALOG / SMELT_DBX_SCHEMA — the dogfood Unity Catalog schema;
#                        default to `workspace`/`smelt_dogfood`.
#   SMELT_DBX_ORACLE_SCHEMA — the full-refresh oracle's Unity Catalog schema
#                        (criterion 8 of the outcome); defaults to
#                        `smelt_dogfood_oracle`.
#
# Mirrors scripts/spark-env.sh: the PyO3-embedded interpreter resolves
# `import smelt.databricks_adapter` and `import databricks.connect` via
# PYTHONPATH, so both the repo's python/ package and the pinned
# `databricks-connect` client venv's site-packages go on it. That venv
# (.smelt-dbx-venv) is DISJOINT from .smelt-spark-venv — the two packages
# conflict — so this script never touches PYSPARK_PYTHON or puts the
# local-Spark venv on PYTHONPATH.
#
# Credential provisioning (the encrypted key, the auth script that decrypts
# it into $SMELT_DBX_CONFIG_DIR/token) is phase 4's job (human-gated, needs
# a real workspace) and does not exist yet. This script only *reads* what
# phase 4 will produce, exactly as scripts/bigquery-env.sh reads
# scripts/bigquery-auth.sh's token file — if the config dir or token are
# absent, the live gate variable simply stays unset.

_dbx_repo_root="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"

export PYTHONPATH="${_dbx_repo_root}/python${PYTHONPATH:+:${PYTHONPATH}}"
_dbx_venv_site="${_dbx_repo_root}/.smelt-dbx-venv/lib/python3.12/site-packages"
export PYTHONPATH="${_dbx_venv_site}:${PYTHONPATH}"
if [ ! -d "${_dbx_venv_site}" ]; then
  echo "no Databricks Connect client venv — create it with: bash scripts/dbx-dogfood-venv.sh" >&2
fi

export SMELT_DBX_CATALOG="${SMELT_DBX_CATALOG:-workspace}"
export SMELT_DBX_SCHEMA="${SMELT_DBX_SCHEMA:-smelt_dogfood}"
export SMELT_DBX_ORACLE_SCHEMA="${SMELT_DBX_ORACLE_SCHEMA:-smelt_dogfood_oracle}"

_dbx_config_dir="${SMELT_DBX_CONFIG_DIR:-$HOME/.config/databricks-smelt-dogfood}"
_dbx_env_file="${_dbx_config_dir}/config.env"
_dbx_token_file="${_dbx_config_dir}/token"

unset SMELT_DBX_HOST
unset SMELT_DBX_HOSTNAME
unset SMELT_DBX_TOKEN

if [ -f "${_dbx_env_file}" ]; then
  # shellcheck disable=SC1090
  . "${_dbx_env_file}"
  export SMELT_DBX_HOST
fi

if [ -n "${SMELT_DBX_HOST:-}" ]; then
  # Strip a leading scheme (`https://`/`http://`) and any trailing slash —
  # the target's `host:` key is a bare hostname
  # (`crates/smelt-core/src/config.rs`'s "no scheme, no trailing slash"
  # validation), while SMELT_DBX_HOST itself stays scheme-bearing for
  # dbx-auth.sh's and the Python query wrapper's own URL-building.
  SMELT_DBX_HOSTNAME="${SMELT_DBX_HOST#http://}"
  SMELT_DBX_HOSTNAME="${SMELT_DBX_HOSTNAME#https://}"
  SMELT_DBX_HOSTNAME="${SMELT_DBX_HOSTNAME%/}"
  export SMELT_DBX_HOSTNAME
fi

if [ -f "${_dbx_token_file}" ]; then
  # The token file is two lines (token, then an epoch expiry stamp — see
  # dbx-auth.sh); `head -n1` takes only the token so it can't pick up an
  # embedded newline that would corrupt the `Authorization: Bearer` header.
  SMELT_DBX_TOKEN="$(head -n1 "${_dbx_token_file}")"
  export SMELT_DBX_TOKEN
fi

echo "dogfood target: ${SMELT_DBX_HOST:-UNSET}.${SMELT_DBX_CATALOG}.${SMELT_DBX_SCHEMA}"
if [ -n "${SMELT_DBX_TOKEN:-}" ]; then
  echo "SMELT_DBX_TOKEN=SET"
else
  echo "SMELT_DBX_TOKEN=UNSET (ambient credentials, if SMELT_DBX_HOST is set)"
fi
