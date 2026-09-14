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
#     bash scripts/dbx-bundle.sh runs list <job-name>  # read-only: jobs list-runs
#     bash scripts/dbx-bundle.sh runs get <run-id>     # read-only: jobs get-run
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
  validate|deploy|run|seed|runs)
    ;;
  *)
    echo "unsupported subcommand '${SUBCOMMAND}' — only validate, deploy, run, seed and runs are wrapped" >&2
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

if [[ "${SUBCOMMAND}" == "runs" ]]; then
  RUNS_ACTION="${1:-}"
  if [[ -z "${RUNS_ACTION}" ]]; then
    echo "usage: $(basename "$0") runs {list <job-name>|get <run-id>}" >&2
    exit 1
  fi
  shift

  case "${RUNS_ACTION}" in
    list)
      JOB_NAME="${1:-}"
      if [[ -z "${JOB_NAME}" ]]; then
        echo "usage: $(basename "$0") runs list <job-name>" >&2
        exit 1
      fi
      JOB_ID="$(DATABRICKS_HOST="${SMELT_DBX_HOST}" DATABRICKS_TOKEN="${SMELT_DBX_TOKEN}" \
        databricks jobs list --output json \
        | python3 -c "import json,sys; jobs=json.load(sys.stdin); matches=[j['job_id'] for j in jobs if j.get('settings',{}).get('name')=='${JOB_NAME}']; print(matches[0])" 2>/dev/null)"
      if [[ -z "${JOB_ID}" ]]; then
        echo "no job found named '${JOB_NAME}'" >&2
        exit 1
      fi
      DATABRICKS_HOST="${SMELT_DBX_HOST}" DATABRICKS_TOKEN="${SMELT_DBX_TOKEN}" \
        exec databricks jobs list-runs --job-id "${JOB_ID}" --output json
      ;;
    get)
      RUN_ID="${1:-}"
      if [[ -z "${RUN_ID}" ]]; then
        echo "usage: $(basename "$0") runs get <run-id>" >&2
        exit 1
      fi
      DATABRICKS_HOST="${SMELT_DBX_HOST}" DATABRICKS_TOKEN="${SMELT_DBX_TOKEN}" \
        exec databricks jobs get-run "${RUN_ID}" --output json
      ;;
    *)
      echo "unsupported runs action '${RUNS_ACTION}' — only list and get are wrapped" >&2
      exit 1
      ;;
  esac
fi

if [[ "${SUBCOMMAND}" == "seed" ]]; then
  CATALOG="${SMELT_DBX_CATALOG:-workspace}"
  SCHEMA="${SMELT_DBX_SCHEMA:-smelt_dogfood}"
  VOLUME="${SMELT_DBX_VOLUME:-smelt_project}"
  # `databricks fs cp` needs the `dbfs:` scheme prefix to address a Unity
  # Catalog Volume path at all — without it the CLI reports the volume's own
  # root as "no such directory" even though the volume exists (measured
  # against CLI v1.16.1, phase 11c).
  VOLUME_PATH="dbfs:/Volumes/${CATALOG}/${SCHEMA}/${VOLUME}/project"

  # .smelt/ (the run-state ledger) is deliberately absent from this list, so
  # re-running seed against an already-deployed project can never clobber it.
  # `functions/` is a project-root convention directory discovered
  # independently of `paths:` (smelt-core/src/workspace.rs), not merely
  # anything reachable under `models/` — omitting it here left
  # `silver.actor_sessions`'s `smelt.functions.sessionize` call unresolved on
  # the deployed Volume (measured phase 11m: `UnknownSmeltFn`), a copy-paste
  # gap from phase 03's original `SEED_ITEMS`, never a real absence locally.
  SEED_ITEMS=(smelt.yml models functions)

  # `fs cp` refuses to create the destination directory implicitly when
  # copying a file into it, so the project/ directory must exist first — a
  # freshly `bundle deploy`-created Volume starts empty.
  DATABRICKS_HOST="${SMELT_DBX_HOST}" DATABRICKS_TOKEN="${SMELT_DBX_TOKEN}" \
    databricks fs mkdir "${VOLUME_PATH}"

  for item in "${SEED_ITEMS[@]}"; do
    DATABRICKS_HOST="${SMELT_DBX_HOST}" DATABRICKS_TOKEN="${SMELT_DBX_TOKEN}" \
      databricks fs cp --recursive --overwrite \
      "${BUNDLE_DIR}/${item}" "${VOLUME_PATH}/${item}"
  done
  echo "seeded ${VOLUME_PATH}"
  exit 0
fi

if [[ "${SUBCOMMAND}" == "deploy" ]]; then
  # The two `smelt_env.dependencies` entries reference the deployed wheel by
  # its exact workspace path (`${workspace.file_path}/dist/<exact
  # filename>`), not a wildcard glob — pip's library installer resolves a
  # `/Workspace/...` requirement as a literal path with no shell-style glob
  # expansion (measured phase 11i: `*_x86_64.whl is not a valid wheel
  # filename`). So the exact filenames must be known before `bundle deploy`
  # runs. Pre-build here (idempotent — `bundle deploy`'s own artifact build
  # step rebuilds identically) purely to read the resulting filenames off
  # disk, then pass them through as bundle variables rather than duplicating
  # `dbx-wheel-build.sh`'s own cp311/manylinux_2_28 spelling in YAML.
  bash "${REPO_ROOT}/scripts/dbx-wheel-build.sh"
  X86_64_WHEEL="$(basename "$(ls "${REPO_ROOT}"/dist/*_x86_64.whl)")"
  AARCH64_WHEEL="$(basename "$(ls "${REPO_ROOT}"/dist/*_aarch64.whl)")"
  DATABRICKS_HOST="${SMELT_DBX_HOST}" DATABRICKS_TOKEN="${SMELT_DBX_TOKEN}" \
    exec databricks bundle deploy --target dogfood \
    --var "x86_64_wheel_name=${X86_64_WHEEL}" \
    --var "aarch64_wheel_name=${AARCH64_WHEEL}" \
    "$@"
fi

DATABRICKS_HOST="${SMELT_DBX_HOST}" DATABRICKS_TOKEN="${SMELT_DBX_TOKEN}" \
  exec databricks bundle "${SUBCOMMAND}" --target dogfood "$@"
