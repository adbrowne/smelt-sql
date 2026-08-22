#!/usr/bin/env bash
# bigquery-probe-quota.sh — the per-table modification limit and the dataset
# creation limit, as measured numbers.
#
#     bash scripts/bigquery-probe-quota.sh [report-path]
#
# A generative conformance sweep modifies one maintained table repeatedly and
# allocates one dataset per case. Both of those touch a BigQuery rate limit, and
# both sizings have to come from the warehouse rather than from documentation.
#
# The measurement discipline that matters here: the spacing between consecutive
# modifications is set by an explicit sleep computed from the *start* of the
# previous request, never by the request's own round-trip time. An earlier
# reading that the per-table limit did not bind came from a loop whose latency
# happened to keep it under the burst threshold; a rate measurement whose
# spacing is its own latency measures nothing.
#
# A refusal is only tolerated when it carries the table-update-quota shape.
# Anything else — auth, syntax, permission, an unrecognised 4xx — fails the
# probe loudly rather than being absorbed into the finding.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
REPORT="${1:-/tmp/bigquery-probe-quota.txt}"

# shellcheck disable=SC1091
source scripts/bigquery-env.sh >/dev/null 2>&1 || true
[[ -n "${SMELT_BQ_ACCESS_TOKEN:-}" ]] || {
  echo "no valid SMELT_BQ_ACCESS_TOKEN — run: bash scripts/bigquery-auth.sh" >&2
  exit 1
}

P="${SMELT_BQ_PROJECT:?SMELT_BQ_PROJECT unset}"
LOC="${SMELT_BQ_LOCATION:?SMELT_BQ_LOCATION unset}"
QUERY_API="https://bigquery.googleapis.com/bigquery/v2/projects/${P}/queries"
DS_API="https://bigquery.googleapis.com/bigquery/v2/projects/${P}/datasets"

# Burst size and the spacings swept. Eight is the burst already known to be
# refused at zero spacing; the sweep's job is to find the spacing at which it
# stops being refused.
BURST="${BQ_PROBE_BURST:-8}"
SPACINGS="${BQ_PROBE_SPACINGS:-0 2 3 5}"
DATASET_BURST="${BQ_PROBE_DATASET_BURST:-10}"

PROBE_DS="${SMELT_BQ_DATASET:-smelt_test}_quotaprobe_$$"

: >"$REPORT"
say() { printf '%s\n' "$*" | tee -a "$REPORT"; }

# Every modification this probe issues counts against the per-table daily cap,
# so the total is reported: a sweep that cannot say how much quota it spent
# cannot say how close a real sweep sits to the cap.
TOTAL_MODS=0

# §B's datasets are siblings of PROBE_DS, not tables inside it, and a dataset
# resource carries no expiration of its own — only the tables in it do. So the
# sweep has to name every dataset the probe could have created, not just the
# one holding §A's tables. Deleting an absent dataset is a no-op, which makes
# this safe to run after a partial or interrupted probe.
cleanup() {
  local i
  for ((i = 1; i <= DATASET_BURST; i++)); do
    curl -sS -X DELETE "${DS_API}/${PROBE_DS}_rate_${i}?deleteContents=true" \
      -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" >/dev/null 2>&1 || true
  done
  curl -sS -X DELETE "${DS_API}/${PROBE_DS}?deleteContents=true" \
    -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# run_query <sql> — echoes "OK" or "ERR<TAB><reason>". Never exits; the caller
# classifies, because only the caller knows which refusals are the finding.
run_query() {
  local sql="$1" resp msg
  resp=$(curl -sS -X POST "$QUERY_API" \
    -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"query\":$(jq -Rs . <<<"$sql"),\"useLegacySql\":false,\"timeoutMs\":60000}")
  msg=$(jq -r '.error.message // (.status.errorResult.message // "")' <<<"$resp")
  if [[ -n "$msg" ]]; then
    printf 'ERR\t%s\n' "$(tr '\n' ' ' <<<"$msg")"
  else
    printf 'OK\n'
  fi
}

# classify <message> — the one tolerated refusal shape, by name. Anything the
# allow-list does not name is a probe failure, not a finding.
classify() {
  local msg="$1"
  if [[ "$msg" == *"exceeded quota for table update operations"* ]] \
    || [[ "$msg" == *"Exceeded rate limits: too many table update operations"* ]]; then
    echo "QUOTA"
  else
    echo "OTHER"
  fi
}

fail_loud() {
  say ""
  say "PROBE FAILED — unrecognised error, not a quota refusal:"
  say "  $1"
  exit 1
}

say "# BigQuery modification and dataset limits — $(date '+%Y-%m-%d %H:%M:%S')"
say "# project=${P} location=${LOC} probe-dataset=${PROBE_DS}"
say ""

# ---------------------------------------------------------------------------
say "## Setup"
r=$(curl -sS -X POST "$DS_API" \
  -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "{\"datasetReference\":{\"projectId\":\"${P}\",\"datasetId\":\"${PROBE_DS}\"},\"location\":\"${LOC}\",\"defaultTableExpirationMs\":\"3600000\"}")
msg=$(jq -r '.error.message // ""' <<<"$r")
[[ -z "$msg" ]] || fail_loud "could not create probe dataset: $msg"
say "  created dataset ${PROBE_DS}"
say ""

# ---------------------------------------------------------------------------
# A: per-table modification spacing.
#
# One fresh table per spacing, so a refusal is attributable to that spacing's
# burst rather than to quota carried over from the previous spacing.
say "## A. Per-table modification rate"
say ""
say "Statement, repeated ${BURST}x against one table name:"
say "    CREATE OR REPLACE TABLE <ds>.<t> AS SELECT <i> AS n"
say "Spacing is measured start-of-request to start-of-request, held by an"
say "explicit sleep; the request's own round trip does not contribute to it."
say ""

BINDING_SPACING="none-of-the-swept-values"
for S in $SPACINGS; do
  TBL="burst_s${S}"
  ok=0
  refused_at=""
  for ((i = 1; i <= BURST; i++)); do
    t0=$(date +%s.%N)
    res=$(run_query "CREATE OR REPLACE TABLE \`${P}.${PROBE_DS}.${TBL}\` AS SELECT ${i} AS n")
    TOTAL_MODS=$((TOTAL_MODS + 1))
    if [[ "$res" == OK* ]]; then
      ok=$((ok + 1))
    else
      m="${res#*$'\t'}"
      cls=$(classify "$m")
      [[ "$cls" == "QUOTA" ]] || fail_loud "spacing=${S}s op=${i}: $m"
      refused_at="${refused_at:-$i}"
    fi
    # Hold the *interval*, not the gap after the response.
    if ((i < BURST)); then
      elapsed=$(awk -v a="$t0" -v b="$(date +%s.%N)" 'BEGIN{printf "%.3f", b-a}')
      rem=$(awk -v s="$S" -v e="$elapsed" 'BEGIN{d=s-e; printf "%.3f", (d>0?d:0)}')
      sleep "$rem"
    fi
  done
  if [[ -z "$refused_at" ]]; then
    say "  spacing=${S}s: ${ok}/${BURST} succeeded, no refusal"
    if [[ "$BINDING_SPACING" == "none-of-the-swept-values" ]]; then
      BINDING_SPACING="${S}s"
    fi
  else
    say "  spacing=${S}s: ${ok}/${BURST} succeeded, first refusal at op ${refused_at} (table-update quota)"
  fi
done
say ""
say "  Smallest swept spacing with no refusal: ${BINDING_SPACING}"
say ""

# ---------------------------------------------------------------------------
# B: dataset create/drop rate.
say "## B. Dataset creation and deletion rate"
say ""
say "Statement pair, repeated ${DATASET_BURST}x back-to-back with no spacing:"
say "    POST   /datasets {datasetId: <ds>_rate_<i>}"
say "    DELETE /datasets/<ds>_rate_<i>?deleteContents=true"
say ""

ds_ok=0
ds_refused=""
for ((i = 1; i <= DATASET_BURST; i++)); do
  D="${PROBE_DS}_rate_${i}"
  r=$(curl -sS -X POST "$DS_API" \
    -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"datasetReference\":{\"projectId\":\"${P}\",\"datasetId\":\"${D}\"},\"location\":\"${LOC}\",\"defaultTableExpirationMs\":\"3600000\"}")
  msg=$(jq -r '.error.message // ""' <<<"$r")
  if [[ -n "$msg" ]]; then
    case "$msg" in
      *"rate limit"* | *"Rate Limit"* | *"rateLimitExceeded"* | *"quota"*)
        ds_refused="${ds_refused:-$i}"
        ;;
      *) fail_loud "dataset create ${i}: $msg" ;;
    esac
  else
    ds_ok=$((ds_ok + 1))
    r=$(curl -sS -X DELETE "${DS_API}/${D}?deleteContents=true" \
      -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
      -H "Content-Type: application/json")
    msg=$(jq -r '.error.message // ""' <<<"$r")
    if [[ -n "$msg" ]]; then
      case "$msg" in
        *"rate limit"* | *"Rate Limit"* | *"rateLimitExceeded"* | *"quota"*)
          ds_refused="${ds_refused:-$i}"
          ;;
        *) fail_loud "dataset delete ${i}: $msg" ;;
      esac
    fi
  fi
done
if [[ -z "$ds_refused" ]]; then
  say "  ${ds_ok}/${DATASET_BURST} create+drop pairs succeeded back-to-back, no refusal"
else
  say "  ${ds_ok}/${DATASET_BURST} succeeded; first rate refusal at pair ${ds_refused}"
fi
say ""

# ---------------------------------------------------------------------------
say "## C. Daily per-table budget consumed by this probe"
say "  ${TOTAL_MODS} modifications, spread over $(wc -w <<<"$SPACINGS") distinct tables"
say "  (max $((TOTAL_MODS / $(wc -w <<<"$SPACINGS"))) against any single table)"
say ""
say "Report written to ${REPORT}"
