#!/usr/bin/env bash
# dbx-auth.sh — turn the encrypted Databricks dogfood credential into a bearer
# token at $CONFIG_DIR/token.
#
#     bash scripts/dbx-auth.sh
#     bash scripts/dbx-auth.sh --self-test
#
# Run this yourself at the start of a Databricks dogfood session:
#
#     bash scripts/dbx-auth.sh
#     source scripts/dbx-dogfood-env.sh
#
# It prompts for the passphrase protecting the encrypted secret, then
# dispatches on SMELT_DBX_CRED_KIND (recorded by scripts/dbx-key.sh):
#
#   oauth-m2m — exchanges the client id/secret at
#               $SMELT_DBX_HOST/oidc/v1/token for a short-lived bearer token,
#               minted fresh on every run.
#   pat       — decrypts the long-lived personal access token as-is.
#
# Either branch ends at the same artefact: $CONFIG_DIR/token (0600), plus an
# expiry stamp this script prints (never the token itself).
#
# --self-test validates argument handling with no workspace and no gpg
# prompt (docs/outcomes/20260912-databricks-dogfood-spine/phases/04a-plan.md).
set -euo pipefail

CONFIG_DIR="${SMELT_DBX_CONFIG_DIR:-$HOME/.config/databricks-smelt-dogfood}"

self_test() {
  local kind
  for kind in oauth-m2m pat; do
    case "$kind" in
      oauth-m2m|pat) : ;;
      *) echo "self-test: dispatch broken for $kind" >&2; exit 1 ;;
    esac
  done
  if dispatch_error_for_unknown_kind "bogus-kind" 2>/dev/null; then
    echo "self-test: expected an unknown credential kind to error" >&2
    exit 1
  fi
  echo "dbx-auth.sh --self-test: OK"
}

# dispatch_error_for_unknown_kind KIND — exercises the same case statement the
# real auth path uses, so --self-test proves the third-value error without a
# workspace, a secret, or a prompt.
dispatch_error_for_unknown_kind() {
  case "$1" in
    oauth-m2m|pat) return 0 ;;
    *) echo "unknown credential kind: $1 (expected oauth-m2m or pat)" >&2; return 1 ;;
  esac
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit 0
fi

SECRET_ENC="$CONFIG_DIR/secret.gpg"
CONFIG_ENV="$CONFIG_DIR/config.env"
TOKEN_FILE="$CONFIG_DIR/token"

[[ -f "$SECRET_ENC" ]] || { echo "no encrypted secret at $SECRET_ENC — run scripts/dbx-key.sh" >&2; exit 1; }
[[ -f "$CONFIG_ENV" ]] || { echo "no config at $CONFIG_ENV — run scripts/dbx-key.sh" >&2; exit 1; }

# shellcheck disable=SC1090
. "$CONFIG_ENV"
dispatch_error_for_unknown_kind "${SMELT_DBX_CRED_KIND:?SMELT_DBX_CRED_KIND missing from $CONFIG_ENV}"

_plain="$(mktemp)"
trap 'command -v shred >/dev/null 2>&1 && shred -u "$_plain" 2>/dev/null || rm -f "$_plain"' EXIT

# --yes is required: mktemp already created the file, and gpg otherwise stops
# to ask about overwriting it. NOT --batch, which would suppress the
# passphrase prompt this deliberately depends on.
gpg --quiet --yes --decrypt --output "$_plain" "$SECRET_ENC"

umask 077
if [[ "$SMELT_DBX_CRED_KIND" == "oauth-m2m" ]]; then
  : "${SMELT_DBX_CLIENT_ID:?SMELT_DBX_CLIENT_ID missing from $CONFIG_ENV for oauth-m2m}"
  : "${SMELT_DBX_HOST:?SMELT_DBX_HOST missing from $CONFIG_ENV}"
  CLIENT_SECRET="$(cat "$_plain")"
  RESP="$(curl -sS -u "${SMELT_DBX_CLIENT_ID}:${CLIENT_SECRET}" \
    -d 'grant_type=client_credentials&scope=all-apis' \
    "${SMELT_DBX_HOST}/oidc/v1/token")"
  TOKEN="$(jq -r '.access_token // empty' <<<"$RESP")"
  EXPIRES_IN="$(jq -r '.expires_in // 3600' <<<"$RESP")"
  [[ -n "$TOKEN" ]] || { echo "token exchange failed: $(jq -r '.error_description // .' <<<"$RESP")" >&2; exit 1; }
  EXPIRES=$(( $(date +%s) + EXPIRES_IN ))
else
  TOKEN="$(cat "$_plain")"
  # PATs are long-lived; record a nominal 1-hour freshness window so
  # dbx-dogfood-env.sh's callers have a consistent "re-run dbx-auth.sh" signal.
  EXPIRES=$(( $(date +%s) + 3500 ))
fi

printf '%s\n%s\n' "$TOKEN" "$EXPIRES" > "$TOKEN_FILE"
chmod 600 "$TOKEN_FILE"

echo "Databricks token ready — valid until $(date -d "@$EXPIRES" '+%H:%M:%S')"
echo "Now run:  source scripts/dbx-dogfood-env.sh"
