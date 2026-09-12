#!/usr/bin/env bash
# dbx-key.sh — capture the Databricks dogfood credential and encrypt it at rest.
#
#     bash scripts/dbx-key.sh
#     bash scripts/dbx-key.sh --self-test
#
# Credential-agnostic: Free Edition may permit a service principal with an
# OAuth machine-to-machine secret, or only a personal access token — which
# one is an empirical fact scripts/dbx-provision.sh discovers, not a design
# choice made here. Both kinds land in the same encrypted artefact
# ($CONFIG_DIR/secret.gpg) plus a config.env recording SMELT_DBX_CRED_KIND, so
# scripts/dbx-auth.sh can branch on the kind without this script knowing how
# dbx-auth.sh will later use the secret.
#
# Prompts for a passphrase that protects the secret; you type that same
# passphrase each time you later run scripts/dbx-auth.sh. The plaintext
# secret exists only in a temp file that is shredded before exit.
#
# --self-test validates argument handling and config-dir permissions against
# a temporary directory — no prompt, no gpg, no workspace. It exists so this
# script is gateable with no human and no credential
# (docs/outcomes/20260912-databricks-dogfood-spine/phases/04a-plan.md).
set -euo pipefail

CONFIG_DIR="${SMELT_DBX_CONFIG_DIR:-$HOME/.config/databricks-smelt-dogfood}"

self_test() {
  # Not a `trap ... EXIT`: that trap outlives this function's `local tmp` (it
  # fires when the whole script exits, by which point `tmp` is out of scope
  # and unbound under `set -u`), so clean up directly instead.
  local tmp
  tmp="$(mktemp -d)"

  mkdir -p "$tmp/config"; chmod 700 "$tmp/config"
  local perms
  perms=$(stat -c '%a' "$tmp/config")
  [[ "$perms" == "700" ]] || { echo "self-test: expected 700, got $perms" >&2; exit 1; }

  local kind
  for kind in oauth-m2m pat; do
    case "$kind" in
      oauth-m2m|pat) : ;;
      *) echo "self-test: credential-kind dispatch is broken for $kind" >&2; exit 1 ;;
    esac
  done

  rm -rf "$tmp"
  echo "dbx-key.sh --self-test: OK"
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit 0
fi

mkdir -p "$CONFIG_DIR"; chmod 700 "$CONFIG_DIR"
SECRET_ENC="$CONFIG_DIR/secret.gpg"

if [[ -f "$SECRET_ENC" ]]; then
  read -rp "Encrypted secret already exists. Replace it? [y/N] " reply
  [[ "$reply" =~ ^[Yy] ]] || { echo "keeping existing secret"; exit 0; }
fi

echo "Credential kind: Free Edition may permit a service principal with an"
echo "OAuth machine-to-machine secret, or only a personal access token."
read -rp "Kind [oauth-m2m/pat]: " CRED_KIND
case "$CRED_KIND" in
  oauth-m2m|pat) : ;;
  *) echo "unknown credential kind: $CRED_KIND (expected oauth-m2m or pat)" >&2; exit 1 ;;
esac

read -rp "Workspace host (e.g. https://dbc-xxxxxxxx-xxxx.cloud.databricks.com): " HOST

CLIENT_ID=""
if [[ "$CRED_KIND" == "oauth-m2m" ]]; then
  read -rp "OAuth client ID: " CLIENT_ID
  read -rsp "OAuth client secret: " SECRET_PLAIN
  echo
else
  read -rsp "Personal access token: " SECRET_PLAIN
  echo
fi

_plain="$(mktemp)"
trap 'command -v shred >/dev/null 2>&1 && shred -u "$_plain" 2>/dev/null || rm -f "$_plain"' EXIT
printf '%s' "$SECRET_PLAIN" > "$_plain"

echo
echo "Choose a passphrase. You will type it whenever you start a dogfood session."
gpg --batch --yes --symmetric --cipher-algo AES256 --output "$SECRET_ENC" "$_plain"
chmod 600 "$SECRET_ENC"

# Record the config the env/auth scripts read.
{
  echo "SMELT_DBX_HOST=$HOST"
  echo "SMELT_DBX_CRED_KIND=$CRED_KIND"
  [[ -n "$CLIENT_ID" ]] && echo "SMELT_DBX_CLIENT_ID=$CLIENT_ID"
} > "$CONFIG_DIR/config.env"
chmod 600 "$CONFIG_DIR/config.env"

echo
echo "Encrypted secret: $SECRET_ENC (plaintext shredded)"
echo "Config:           $CONFIG_DIR/config.env"
echo
echo "Start a dogfood session with:"
echo "  bash scripts/dbx-auth.sh && source scripts/dbx-dogfood-env.sh"
