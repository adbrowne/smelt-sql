# shellcheck shell=bash  # sourced, not executed — no shebang by design
# bq-dogfood-env.sh — source this (`source scripts/bq-dogfood-env.sh`) to point
# the github_activity dogfood pipeline at the long-lived BigQuery dataset.
#
#     source scripts/bq-dogfood-env.sh
#     smelt run --target bigquery ...
#
# It layers on scripts/bigquery-env.sh — which is where PYTHONPATH, the client
# venv and the adapter's contract live — and then changes exactly two things.
#
# 1. THE TABLE-EXPIRATION GUARD. bigquery-env.sh sets
#    SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS unconditionally (2h by default), and
#    python/smelt/bigquery_adapter.py stamps whatever it finds there onto any
#    dataset it creates. For the integration suites that is a feature: a run
#    killed by a panic or Ctrl-C sheds its tables without teardown having to
#    run. For the dogfood pipeline it is the opposite of what is wanted — the
#    whole point is a dataset that accumulates history across days, and the
#    equivalence invariant is checked against state that must still be there
#    tomorrow.
#
#    The adapter calls create_dataset(..., exists_ok=True), which does NOT
#    modify a dataset that already exists, so as long as smelt_dogfood is
#    present the variable never actually reaches it. That mitigation is real
#    but undocumented and load-bearing, and the failure it stands between us
#    and — history silently evaporating a couple of hours later, looking like
#    nothing at the time — is bad enough that relying on it is not sensible.
#    So it is unset here, explicitly.
#
# 2. THE TARGET. bigquery-env.sh reads project/dataset from the TEST project's
#    isolated config (~/.config/gcloud-smelt-bq/config.env). The dogfood
#    pipeline uses the same project but a different, no-expiry dataset, and
#    authenticates through ADC impersonation rather than that config's
#    short-lived token — so SMELT_BQ_DATASET is overridden after the fact.
#
# Credentials: none are set here. The dogfood path authenticates with
# Application Default Credentials impersonating
# smelt-dogfood@smelt-bq-test-20260816.iam.gserviceaccount.com — see
# docs/outcomes/20260906-bigquery-dogfood-spine/phases/07-plan.md.

_dogfood_repo_root="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"

# shellcheck source=scripts/bigquery-env.sh
. "${_dogfood_repo_root}/scripts/bigquery-env.sh"

# (1) — see the note above. This must come AFTER the source, which sets it.
unset SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS

# (2) — same project, different dataset. No default table expiration.
export SMELT_BQ_PROJECT="${SMELT_BQ_PROJECT:-smelt-bq-test-20260816}"
export SMELT_BQ_DATASET="smelt_dogfood"
export SMELT_BQ_LOCATION="${SMELT_BQ_LOCATION:-US}"

# The token minted for the TEST project is not the dogfood credential and must
# not leak into this shell — the dogfood path uses ADC impersonation instead.
unset SMELT_BQ_ACCESS_TOKEN

echo "dogfood target: ${SMELT_BQ_PROJECT}.${SMELT_BQ_DATASET} (${SMELT_BQ_LOCATION})"
echo "SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS=${SMELT_BQ_DEFAULT_TABLE_EXPIRATION_MS-UNSET} (tables must persist)"
