#!/usr/bin/env bash
# bigquery-auth.sh — mint a SHORT-LIVED (1 hour) BigQuery access token.
#
# Run this yourself at the start of a BigQuery test session:
#
#     bash scripts/bigquery-auth.sh
#
# It prompts for the passphrase protecting the service-account key, mints an
# OAuth access token, and writes it to $BQ_CONFIG_DIR/token with an expiry
# stamp. The plaintext key is never written to disk.
#
# Outside the one-hour window there is no usable credential on this machine,
# and minting a new one requires a human to type the passphrase.
set -euo pipefail

BQ_CONFIG_DIR="${SMELT_BQ_CONFIG_DIR:-$HOME/.config/gcloud-smelt-bq}"
KEY_ENC="$BQ_CONFIG_DIR/sa-key.json.gpg"
TOKEN_FILE="$BQ_CONFIG_DIR/token"

[[ -f "$KEY_ENC" ]] || { echo "no encrypted key at $KEY_ENC — run scripts/bigquery-key.sh" >&2; exit 1; }

export CLOUDSDK_CONFIG="$BQ_CONFIG_DIR"
# `bq`/`gcloud` must be resolvable by name, not just absolute path.
[[ -d "$HOME/google-cloud-sdk/bin" ]] && export PATH="$HOME/google-cloud-sdk/bin:$PATH"
command -v gcloud >/dev/null 2>&1 || { echo "gcloud not found on PATH" >&2; exit 1; }

_plain="$(mktemp)"
trap 'command -v shred >/dev/null 2>&1 && shred -u "$_plain" 2>/dev/null || rm -f "$_plain"' EXIT

# --yes is required: mktemp already created the file, and gpg otherwise stops to
# ask about overwriting it. NOT --batch, which would suppress the passphrase
# prompt this deliberately depends on.
gpg --quiet --yes --decrypt --output "$_plain" "$KEY_ENC"

gcloud auth activate-service-account --key-file="$_plain" --quiet >/dev/null 2>&1

TOKEN="$(gcloud auth print-access-token)"
EXPIRES=$(( $(date +%s) + 3500 ))

umask 077
printf '%s\n%s\n' "$TOKEN" "$EXPIRES" > "$TOKEN_FILE"
chmod 600 "$TOKEN_FILE"

echo "BigQuery token minted — valid until $(date -d "@$EXPIRES" '+%H:%M:%S')"
echo "Now run:  source scripts/bigquery-env.sh"
