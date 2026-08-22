#!/usr/bin/env bash
# bigquery-login.sh — sign in to the ISOLATED gcloud config for smelt's
# BigQuery tests.
#
#     bash scripts/bigquery-login.sh
#
# Credentials land in ~/.config/gcloud-smelt-bq, never the ambient
# ~/.config/gcloud that every Google client library probes by default. Opens a
# browser; sign in with the account that owns your billing.
set -euo pipefail

export CLOUDSDK_CONFIG="${SMELT_BQ_CONFIG_DIR:-$HOME/.config/gcloud-smelt-bq}"
mkdir -p "$CLOUDSDK_CONFIG"
chmod 700 "$CLOUDSDK_CONFIG"

GCLOUD="$(command -v gcloud || echo "$HOME/google-cloud-sdk/bin/gcloud")"
[[ -x "$GCLOUD" ]] || { echo "gcloud not found — install it first" >&2; exit 1; }

echo "Signing in to isolated config: $CLOUDSDK_CONFIG"
"$GCLOUD" auth login

echo
echo "Active account(s):"
"$GCLOUD" auth list --filter=status:ACTIVE --format='value(account)'
