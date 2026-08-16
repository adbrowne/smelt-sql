#!/usr/bin/env bash
# bigquery-provision.sh — non-interactive provisioning for smelt's BigQuery
# tests, run AFTER `bash scripts/bigquery-login.sh`.
#
#     bash scripts/bigquery-provision.sh <project-id> <billing-account-id>
#
# Creates (idempotently): the BigQuery + IAM APIs, a test dataset whose tables
# self-expire after 24h, a least-privilege service account (project-scoped
# bigquery.user — jobs plus datasets it creates itself — and dataset-scoped
# WRITER on the one test dataset), and a budget alert.
#
# It does NOT mint or encrypt the service-account key — that step needs a
# human-typed passphrase and lives in scripts/bigquery-setup.sh.
set -euo pipefail

PROJECT="${1:?usage: bigquery-provision.sh <project-id> <billing-account-id>}"
BILLING="${2:?usage: bigquery-provision.sh <project-id> <billing-account-id>}"
DATASET="${SMELT_BQ_DATASET:-smelt_test}"
LOCATION="${SMELT_BQ_LOCATION:-US}"

export CLOUDSDK_CONFIG="${SMELT_BQ_CONFIG_DIR:-$HOME/.config/gcloud-smelt-bq}"
# `bq` shells out to `gcloud` for auth, so the SDK bin dir must be on PATH —
# resolving the binaries by absolute path alone is not enough.
[[ -d "$HOME/google-cloud-sdk/bin" ]] && export PATH="$HOME/google-cloud-sdk/bin:$PATH"
GCLOUD="$(command -v gcloud || echo "$HOME/google-cloud-sdk/bin/gcloud")"
BQ="$(command -v bq || echo "$HOME/google-cloud-sdk/bin/bq")"
SA="smelt-bq-test@${PROJECT}.iam.gserviceaccount.com"

say() { printf '\n=== %s\n' "$1"; }

say "Enabling APIs"
"$GCLOUD" services enable bigquery.googleapis.com --project="$PROJECT"
"$GCLOUD" services enable iam.googleapis.com --project="$PROJECT"

say "Dataset $DATASET (tables self-expire after 24h)"
if "$BQ" --project_id="$PROJECT" show --dataset "${PROJECT}:${DATASET}" >/dev/null 2>&1; then
  echo "already exists"
else
  "$BQ" --project_id="$PROJECT" mk --dataset \
     --location="$LOCATION" \
     --default_table_expiration=86400 \
     --description="smelt integration tests — tables expire after 24h" \
     "${PROJECT}:${DATASET}"
fi

say "Control dataset ${DATASET}_notgranted (negative-test target)"
# Exists so the least-privilege check has something real to be refused FROM.
# Querying a non-existent dataset returns "not found" for every principal
# including owners, which proves nothing about the grant.
if "$BQ" --project_id="$PROJECT" show --dataset "${PROJECT}:${DATASET}_notgranted" >/dev/null 2>&1; then
  echo "already exists"
else
  "$BQ" --project_id="$PROJECT" mk --dataset \
     --location="$LOCATION" \
     --description="control dataset: the test service account must NOT have access" \
     "${PROJECT}:${DATASET}_notgranted"
fi

say "Service account $SA"
if "$GCLOUD" iam service-accounts describe "$SA" --project="$PROJECT" >/dev/null 2>&1; then
  echo "already exists"
else
  "$GCLOUD" iam service-accounts create smelt-bq-test \
     --project="$PROJECT" --display-name="smelt integration tests"
fi

say "Grant: roles/bigquery.user at PROJECT scope (run jobs, create own datasets)"
# bigquery.user = jobUser plus bigquery.datasets.create. The test suites isolate
# each run in its own dataset, which jobUser alone cannot create.
#
# This stays least-privilege in the way that matters: the role confers NO access
# to datasets the service account did not create. It becomes OWNER of datasets
# it creates itself and nothing else, so writing to a pre-existing dataset it was
# never granted is still refused — which is exactly what the negative check in
# bigquery-verify.sh asserts, and why that check remains meaningful under it.
"$GCLOUD" projects add-iam-policy-binding "$PROJECT" \
   --member="serviceAccount:${SA}" \
   --role="roles/bigquery.user" \
   --condition=None >/dev/null
echo "granted"

# Drop the narrower jobUser binding if an earlier run left one; bigquery.user
# supersedes it, and leaving both makes the effective grant harder to read.
if "$GCLOUD" projects remove-iam-policy-binding "$PROJECT" \
     --member="serviceAccount:${SA}" \
     --role="roles/bigquery.jobUser" \
     --condition=None >/dev/null 2>&1; then
  echo "removed superseded roles/bigquery.jobUser binding"
fi

say "Grant: WRITER on dataset $DATASET ONLY (not project-wide)"
_cur="$(mktemp)"; _new="$(mktemp)"
trap 'rm -f "$_cur" "$_new"' EXIT
"$BQ" --project_id="$PROJECT" show --format=prettyjson --dataset "${PROJECT}:${DATASET}" > "$_cur"
jq --arg sa "$SA" \
   '.access += [{"role":"WRITER","userByEmail":$sa}] | .access |= unique' \
   "$_cur" > "$_new"
"$BQ" --project_id="$PROJECT" update --source "$_new" "${PROJECT}:${DATASET}"

say "Budget alert (US\$5/month — ALERTS, does not hard-stop spend)"
"$GCLOUD" services enable billingbudgets.googleapis.com --project="$PROJECT" >/dev/null 2>&1 || true
if "$GCLOUD" billing budgets create \
     --billing-account="$BILLING" \
     --display-name="smelt-bq-test cap" \
     --budget-amount=5USD \
     --filter-projects="projects/${PROJECT}" >/dev/null 2>&1; then
  echo "created"
else
  echo "SKIPPED — create one at https://console.cloud.google.com/billing/budgets"
fi

say "Effective dataset ACL"
"$BQ" --project_id="$PROJECT" show --format=prettyjson --dataset "${PROJECT}:${DATASET}" \
  | jq -r '.access[] | "\(.role)\t\(.userByEmail // .specialGroup // .groupByEmail // "-")"'

say "Done — next: bash scripts/bigquery-setup.sh (mints + encrypts the key)"
