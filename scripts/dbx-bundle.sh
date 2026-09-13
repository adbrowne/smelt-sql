#!/usr/bin/env bash
# dbx-bundle.sh — the only caller of `databricks bundle validate`/`deploy`/
# `run` for examples/github_activity's Asset Bundle (criterion 11,
# docs/outcomes/20260912-databricks-dogfood-spine/outcome.md). Mirrors
# scripts/dbx-verify.sh's shape: a thin, allow-listed wrapper so a Claude
# session can drive the bundle without a raw `databricks` invocation needing
# per-call approval.
#
#     bash scripts/dbx-bundle.sh validate            # no workspace needed
#     bash scripts/dbx-bundle.sh deploy               # needs SMELT_DBX_HOST
#     bash scripts/dbx-bundle.sh run github_activity_daily
#     bash scripts/dbx-bundle.sh seed                 # needs SMELT_DBX_HOST
#
# `validate` never needs a workspace or a credential — it is a pure schema
# check against the committed YAML plus the referenced local files, which is
# exactly why it is the per-PR gate (crates/smelt-cli/tests/
# databricks_bundle.rs). `deploy` and `run` need a workspace: they read
# scripts/dbx-dogfood-env.sh's SMELT_DBX_HOST to fill the bundle's `host`
# variable, and fail with the CLI's own error if the workspace is
# unreachable.
#
# `seed` copies smelt.yml and models/ onto the Volume `resources/volume.yml`
# declares (docs/outcomes/20260912-databricks-dogfood-spine/phases/
# 11b-plan.md) — never `.smelt/`, the run-state ledger a re-seed must not
# clobber, since deleting it would silently reset every model's incremental
# state back to a fresh checkout.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
BUNDLE_DIR="${REPO_ROOT}/examples/github_activity"

if ! command -v databricks >/dev/null 2>&1; then
  echo "databricks CLI not found on PATH — run: mise run setup-databricks" >&2
  exit 1
fi

SUBCOMMAND="${1:-}"
if [[ -z "${SUBCOMMAND}" ]]; then
  echo "usage: $(basename "$0") {validate|deploy|run|seed} [args...]" >&2
  exit 1
fi
shift

case "${SUBCOMMAND}" in
  validate|deploy|run|seed)
    ;;
  *)
    echo "unsupported subcommand '${SUBCOMMAND}' — only validate, deploy, run and seed are wrapped" >&2
    exit 1
    ;;
esac

cd "${BUNDLE_DIR}"

if [[ "${SUBCOMMAND}" == "validate" ]]; then
  # `bundle validate` unconditionally calls SCIM Me plus workspace get-status/
  # mkdirs (the CLI's PopulateCurrentUser mutator and root-path bootstrap run
  # on every bundle command, regardless of what the config references) — so
  # "no workspace needed" means a local stub answering those three calls,
  # not a real credential. See scripts/dbx_bundle_validate_stub.py.
  STUB_LOG="$(mktemp)"
  python3 "${REPO_ROOT}/scripts/dbx_bundle_validate_stub.py" >"${STUB_LOG}" &
  STUB_PID=$!
  trap 'kill "${STUB_PID}" 2>/dev/null || true; rm -f "${STUB_LOG}"' EXIT
  for _ in $(seq 1 50); do
    [[ -s "${STUB_LOG}" ]] && break
    sleep 0.1
  done
  STUB_PORT="$(cat "${STUB_LOG}")"
  if [[ -z "${STUB_PORT}" ]]; then
    echo "dbx_bundle_validate_stub.py never printed a port" >&2
    exit 1
  fi
  DATABRICKS_HOST="http://127.0.0.1:${STUB_PORT}" DATABRICKS_TOKEN="stub" \
    databricks bundle validate --target dogfood "$@"
  exit $?
fi

if [[ -z "${SMELT_DBX_HOST:-}" ]]; then
  echo "SMELT_DBX_HOST is not set — source scripts/dbx-dogfood-env.sh first" >&2
  exit 1
fi

if [[ -z "${SMELT_DBX_TOKEN:-}" ]]; then
  echo "SMELT_DBX_TOKEN is not set — run scripts/dbx-auth.sh then source scripts/dbx-dogfood-env.sh first" >&2
  exit 1
fi

if [[ "${SUBCOMMAND}" == "seed" ]]; then
  CATALOG="${SMELT_DBX_CATALOG:-workspace}"
  SCHEMA="${SMELT_DBX_SCHEMA:-smelt_dogfood}"
  VOLUME="${SMELT_DBX_VOLUME:-smelt_project}"
  VOLUME_PATH="/Volumes/${CATALOG}/${SCHEMA}/${VOLUME}/project"

  # Only these two names are copied — .smelt/ (the run-state ledger) is
  # deliberately absent from this list, so re-running seed against an
  # already-deployed project can never clobber it.
  SEED_ITEMS=(smelt.yml models)

  for item in "${SEED_ITEMS[@]}"; do
    DATABRICKS_HOST="${SMELT_DBX_HOST}" DATABRICKS_TOKEN="${SMELT_DBX_TOKEN}" \
      databricks fs cp --recursive --overwrite \
      "${BUNDLE_DIR}/${item}" "${VOLUME_PATH}/${item}"
  done
  echo "seeded ${VOLUME_PATH}"
  exit 0
fi

DATABRICKS_HOST="${SMELT_DBX_HOST}" DATABRICKS_TOKEN="${SMELT_DBX_TOKEN}" \
  exec databricks bundle "${SUBCOMMAND}" --target dogfood "$@"
