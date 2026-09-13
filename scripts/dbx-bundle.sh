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
#
# `validate` never needs a workspace or a credential — it is a pure schema
# check against the committed YAML plus the referenced local files, which is
# exactly why it is the per-PR gate (crates/smelt-cli/tests/
# databricks_bundle.rs). `deploy` and `run` need a workspace: they read
# scripts/dbx-dogfood-env.sh's SMELT_DBX_HOST to fill the bundle's `host`
# variable, and fail with the CLI's own error if the workspace is
# unreachable.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
BUNDLE_DIR="${REPO_ROOT}/examples/github_activity"

if ! command -v databricks >/dev/null 2>&1; then
  echo "databricks CLI not found on PATH — run: mise run setup-databricks" >&2
  exit 1
fi

SUBCOMMAND="${1:-}"
if [[ -z "${SUBCOMMAND}" ]]; then
  echo "usage: $(basename "$0") {validate|deploy|run} [args...]" >&2
  exit 1
fi
shift

case "${SUBCOMMAND}" in
  validate|deploy|run)
    ;;
  *)
    echo "unsupported subcommand '${SUBCOMMAND}' — only validate, deploy and run are wrapped" >&2
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

DATABRICKS_HOST="${SMELT_DBX_HOST}" \
  exec databricks bundle "${SUBCOMMAND}" --target dogfood "$@"
