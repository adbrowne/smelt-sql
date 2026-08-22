#!/usr/bin/env bash
# bigquery-key.sh — mint the service-account key and encrypt it at rest.
#
#     bash scripts/bigquery-key.sh <project-id>
#
# Run once, after scripts/bigquery-provision.sh. Prompts for a passphrase that
# protects the key; you type that same passphrase each time you later run
# scripts/bigquery-auth.sh to mint a 1-hour token.
#
# The plaintext key exists only in a temp file that is shredded before exit.
set -euo pipefail

PROJECT="${1:?usage: bigquery-key.sh <project-id>}"
BQ_CONFIG_DIR="${SMELT_BQ_CONFIG_DIR:-$HOME/.config/gcloud-smelt-bq}"
KEY_ENC="$BQ_CONFIG_DIR/sa-key.json.gpg"
SA="smelt-bq-test@${PROJECT}.iam.gserviceaccount.com"

export CLOUDSDK_CONFIG="$BQ_CONFIG_DIR"
[[ -d "$HOME/google-cloud-sdk/bin" ]] && export PATH="$HOME/google-cloud-sdk/bin:$PATH"

mkdir -p "$BQ_CONFIG_DIR"; chmod 700 "$BQ_CONFIG_DIR"

if [[ -f "$KEY_ENC" ]]; then
  read -rp "Encrypted key already exists. Replace it? [y/N] " reply
  [[ "$reply" =~ ^[Yy] ]] || { echo "keeping existing key"; exit 0; }
fi

_plain="$(mktemp)"
trap 'command -v shred >/dev/null 2>&1 && shred -u "$_plain" 2>/dev/null || rm -f "$_plain"' EXIT

echo "Minting a key for $SA …"
gcloud iam service-accounts keys create "$_plain" \
  --iam-account="$SA" --project="$PROJECT"

echo
echo "Choose a passphrase. You will type it whenever you start a test session."
gpg --batch --yes --symmetric --cipher-algo AES256 --output "$KEY_ENC" "$_plain"
chmod 600 "$KEY_ENC"

# Record the config the env script reads.
{
  echo "SMELT_BQ_PROJECT=$PROJECT"
  echo "SMELT_BQ_DATASET=${SMELT_BQ_DATASET:-smelt_test}"
  echo "SMELT_BQ_LOCATION=${SMELT_BQ_LOCATION:-US}"
  echo "SMELT_BQ_SA=$SA"
} > "$BQ_CONFIG_DIR/config.env"
chmod 600 "$BQ_CONFIG_DIR/config.env"

echo
echo "Encrypted key: $KEY_ENC (plaintext shredded)"
echo "Config:        $BQ_CONFIG_DIR/config.env"
echo
echo "Start a test session with:"
echo "  bash scripts/bigquery-auth.sh && source scripts/bigquery-env.sh"
