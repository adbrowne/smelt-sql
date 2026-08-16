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
[[ -d "$HOME/google-cloud-sdk/bin" ]] && export PATH="$HOME/google-cloud-sdk/bin:$PATH"
GCLOUD="$(command -v gcloud || echo "$HOME/google-cloud-sdk/bin/gcloud")"
SA="smelt-bq-test@${PROJECT}.iam.gserviceaccount.com"

say() { printf '\n=== %s\n' "$1"; }

# Provisioning is a human-identity operation: it enables APIs and edits IAM,
# neither of which the service account it creates can (or should) do.
#
# The active account in this config cannot be trusted to be the human one.
# `bigquery-auth.sh` activates the service account to mint each token, which
# sets it as the config's active account — so after any token mint, an
# unqualified gcloud call here would run as the service account and fail with
# PERMISSION_DENIED. Pin the admin identity explicitly instead.
#
# CLOUDSDK_CORE_ACCOUNT is used rather than `gcloud config set account` because
# it mutates no stored state — this script leaves the config as it found it.
ADMIN_ACCOUNT="${SMELT_BQ_ADMIN_ACCOUNT:-$("$GCLOUD" auth list \
  --format='value(account)' 2>/dev/null | grep -v 'gserviceaccount\.com' | head -n1)}"

if [[ -z "$ADMIN_ACCOUNT" ]]; then
  echo "No human account is signed in to this gcloud config." >&2
  echo "Provisioning enables APIs and edits IAM, which the test service account cannot do." >&2
  echo "Run: bash scripts/bigquery-login.sh" >&2
  echo "(or set SMELT_BQ_ADMIN_ACCOUNT=<your-email> if you signed in elsewhere)" >&2
  exit 1
fi
export CLOUDSDK_CORE_ACCOUNT="$ADMIN_ACCOUNT"
say "Provisioning as $ADMIN_ACCOUNT"

say "Enabling APIs"
"$GCLOUD" services enable bigquery.googleapis.com --project="$PROJECT"
"$GCLOUD" services enable iam.googleapis.com --project="$PROJECT"

# Dataset work goes through the REST API rather than `bq`.
#
# `bq` runs on the SDK's bundled Python and imports pyOpenSSL, a pairing that
# breaks outright on some installs ("module 'lib' has no attribute 'GEN_EMAIL'"
# — a pyOpenSSL/cryptography version mismatch). `gcloud` itself does not touch
# that import path and keeps working, so a broken `bq` would otherwise take down
# provisioning for a reason unrelated to BigQuery. Every other script here
# (bigquery-verify.sh, the probes) already speaks REST over curl, so this also
# leaves one way of talking to BigQuery instead of two.
API="https://bigquery.googleapis.com/bigquery/v2"
TOKEN="$("$GCLOUD" auth print-access-token)"

# HTTP status only — for existence checks.
bq_status() {
  curl -sS -o /dev/null -w '%{http_code}' \
    -H "Authorization: Bearer ${TOKEN}" "${API}/$1"
}

# Body, with a non-zero exit and the API's own message on failure.
bq_call() {
  local method="$1" path="$2" body="${3:-}" resp msg
  if [[ -n "$body" ]]; then
    resp=$(curl -sS -X "$method" -H "Authorization: Bearer ${TOKEN}" \
      -H "Content-Type: application/json" -d "$body" "${API}/${path}")
  else
    resp=$(curl -sS -X "$method" -H "Authorization: Bearer ${TOKEN}" "${API}/${path}")
  fi
  msg=$(jq -r '.error.message // ""' <<<"$resp")
  if [[ -n "$msg" ]]; then
    echo "API ${method} ${path} failed: ${msg}" >&2
    return 1
  fi
  printf '%s' "$resp"
}

ensure_dataset() {
  local id="$1" description="$2" expiration_ms="${3:-}" body
  if [[ "$(bq_status "projects/${PROJECT}/datasets/${id}")" == "200" ]]; then
    echo "already exists"
    return 0
  fi
  body=$(jq -n \
    --arg p "$PROJECT" --arg d "$id" --arg loc "$LOCATION" --arg desc "$description" \
    --arg exp "$expiration_ms" \
    '{datasetReference: {projectId: $p, datasetId: $d}, location: $loc, description: $desc}
     + (if $exp == "" then {} else {defaultTableExpirationMs: $exp} end)')
  bq_call POST "projects/${PROJECT}/datasets" "$body" >/dev/null
  echo "created"
}

say "Dataset $DATASET (tables self-expire after 24h)"
ensure_dataset "$DATASET" "smelt integration tests — tables expire after 24h" 86400000

say "Control dataset ${DATASET}_notgranted (negative-test target)"
# Exists so the least-privilege check has something real to be refused FROM.
# Creating a table in a non-existent dataset returns "Not found" for every
# principal including owners, which proves nothing about the grant.
ensure_dataset "${DATASET}_notgranted" \
  "control dataset: the test service account must NOT have access"

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
# Read-modify-write the dataset's ACL: PATCH replaces the `access` array
# wholesale, so the existing entries have to be carried forward or the owner
# loses access.
_acl=$(bq_call GET "projects/${PROJECT}/datasets/${DATASET}" \
  | jq --arg sa "$SA" \
      '{access: ((.access // []) + [{"role":"WRITER","userByEmail":$sa}] | unique)}')
bq_call PATCH "projects/${PROJECT}/datasets/${DATASET}" "$_acl" >/dev/null
echo "granted"

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
bq_call GET "projects/${PROJECT}/datasets/${DATASET}" \
  | jq -r '.access[] | "\(.role)\t\(.userByEmail // .specialGroup // .groupByEmail // "-")"'

say "Done — next: bash scripts/bigquery-setup.sh (mints + encrypts the key)"
