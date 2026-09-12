#!/usr/bin/env bash
# bq-dogfood-oracle-dataset.sh — create or drop `smelt_dogfood_oracle`, the
# scratch dataset the full-refresh oracle writes into while the equivalence
# invariant is checked on BigQuery (`docs/specs/incremental_models.md`
# §"The equivalence invariant").
#
#     bash scripts/bq-dogfood-oracle-dataset.sh create
#     bash scripts/bq-dogfood-oracle-dataset.sh drop
#
# WHY THIS IS A SEPARATE SCRIPT, RUN UNDER A DIFFERENT CREDENTIAL.
# `scripts/bq-dogfood-provision.sh` deliberately gave the dogfood service
# account only `roles/bigquery.jobUser` at project scope plus WRITER on one
# dataset — "writes to one long-lived dataset and never creates datasets of its
# own". So the SA cannot create or delete a dataset, by design, and a `CREATE
# SCHEMA` issued under its token fails with
#
#     Access Denied: ... does not have bigquery.datasets.create permission
#
# That constraint is worth keeping rather than relaxing: the pipeline's runtime
# credential should not be able to conjure or destroy datasets. So dataset
# lifecycle happens here, under the human credential (`gcloud auth
# print-access-token`, i.e. the project owner), exactly as the original
# provisioning did — and the run itself stays on the scoped SA.
#
# The dataset is created with NO default table expiration and is scaffolding,
# not history: drop it once a sweep has been recorded. `smelt_dogfood` and the
# two source tables are never touched by this script.
set -euo pipefail

# The dogfood path is ADC in the default config. An inherited CLOUDSDK_CONFIG
# would act against the test project's isolated config instead.
unset CLOUDSDK_CONFIG
[[ -d "$HOME/google-cloud-sdk/bin" ]] && export PATH="$HOME/google-cloud-sdk/bin:$PATH"
GCLOUD="$(command -v gcloud || echo "$HOME/google-cloud-sdk/bin/gcloud")"

API="https://bigquery.googleapis.com/bigquery/v2"
PROJECT="${SMELT_BQ_PROJECT:-smelt-bq-test-20260816}"
DATASET="${PARITY_ORACLE_DATASET:-smelt_dogfood_oracle}"
LOCATION="${SMELT_BQ_LOCATION:-US}"
SA="${SMELT_BQ_DOGFOOD_SA:-smelt-dogfood@${PROJECT}.iam.gserviceaccount.com}"

# Never let a caller point this at the dataset holding the pipeline's state or
# its sources. This script deletes datasets; that is the whole blast radius.
case "$DATASET" in
  smelt_dogfood | smelt_test | "")
    echo "REFUSING: '$DATASET' is not a scratch oracle dataset" >&2
    exit 1
    ;;
esac

tok() { "$GCLOUD" auth print-access-token; }

exists() {
  local code
  code="$(curl -sS -o /dev/null -w '%{http_code}' \
    -H "Authorization: Bearer $(tok)" "${API}/projects/${PROJECT}/datasets/${DATASET}")"
  [[ "$code" == "200" ]]
}

do_create() {
  if exists; then
    echo "${DATASET} already exists — leaving it alone"
  else
    local body
    body=$(jq -n --arg p "$PROJECT" --arg d "$DATASET" --arg loc "$LOCATION" \
      '{datasetReference: {projectId: $p, datasetId: $d}, location: $loc,
        description: "smelt dogfood full-refresh ORACLE — scaffolding for the equivalence-invariant sweep; drop when the sweep is recorded"}')
    curl -sS -X POST -H "Authorization: Bearer $(tok)" -H "Content-Type: application/json" \
      -d "$body" "${API}/projects/${PROJECT}/datasets" \
      | jq -r '.datasetReference.datasetId // ("FAILED: " + .error.message)'
  fi

  # Read the expiry back: a successful create is not evidence.
  echo -n "defaultTableExpirationMs = "
  curl -sS -H "Authorization: Bearer $(tok)" \
    "${API}/projects/${PROJECT}/datasets/${DATASET}" \
    | jq -r '.defaultTableExpirationMs // "ABSENT"'

  # The run itself uses the scoped SA, so it needs WRITER here. PATCH replaces
  # the access array wholesale, so existing entries are read and carried
  # forward.
  local acl
  acl=$(curl -sS -H "Authorization: Bearer $(tok)" \
          "${API}/projects/${PROJECT}/datasets/${DATASET}" \
        | jq --arg sa "$SA" '{access: ((.access // []) + [{"role":"WRITER","userByEmail":$sa}] | unique)}')
  curl -sS -X PATCH -H "Authorization: Bearer $(tok)" -H "Content-Type: application/json" \
    -d "$acl" "${API}/projects/${PROJECT}/datasets/${DATASET}" \
    | jq -r '.access[] | "  \(.role) \(.userByEmail // .specialGroup // .groupByEmail // "?")"'
}

do_drop() {
  if ! exists; then
    echo "${DATASET} is already gone"
    return
  fi
  curl -sS -X DELETE -H "Authorization: Bearer $(tok)" \
    "${API}/projects/${PROJECT}/datasets/${DATASET}?deleteContents=true" \
    | jq -r 'if . == {} or . == null then "deleted" else (.error.message // "deleted") end' 2>/dev/null \
    || echo "deleted"
  if exists; then
    echo "STILL PRESENT: ${DATASET}" >&2
    exit 1
  fi
  echo "confirmed gone: ${PROJECT}.${DATASET}"
}

case "${1:-}" in
  create) do_create ;;
  drop) do_drop ;;
  *)
    echo "usage: $0 {create|drop}" >&2
    exit 2
    ;;
esac
