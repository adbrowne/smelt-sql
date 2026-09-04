#!/usr/bin/env bash
#
# Outcome loop — headless driver for outcome-driven work (docs/outcome_loop.md).
#
# Parallel to autonomy-loop.sh, but the unit of work alternates between two
# step types chosen from the active outcome's phase table. The active outcome
# is the first entry in `.claude/outcome-backlog` (ordered outcome
# directories, one per line) whose outcome.md **Status:** is neither `done`
# nor `blocked`:
#
#   first non-done/non-blocked row is `pending`  -> PLAN step   (MODEL_PLAN, default opus)
#   first non-done/non-blocked row is `planned`  -> IMPLEMENT step (MODEL_IMPL, default sonnet)
#
# The PLAN step reads the previous phase's summary, may reshape the remaining
# phase list (never deferring work that serves the outcome's success criteria),
# writes phases/NN-plan.md, flips the row to `planned`. The IMPLEMENT step
# executes phases/NN-plan.md red-green, writes phases/NN-summary.md, flips the
# row to `done`. Prompts: .claude/outcome-plan-prompt.txt /
# .claude/outcome-implement-prompt.txt.
#
# Sentinels (grepped from the final .result only):
#   <<PLAN_READY>>        continue (next iteration implements)
#   <<PHASE_COMPLETE>>    continue (next iteration plans the next phase)
#   <<PHASE_BLOCKED>>     recorded in outcome.md; continue to next row
#   <<OUTCOME_COMPLETE>>  exit 0
#   <<OUTCOME_BLOCKED>>   exit 2 (needs a human)
#
# Stop gracefully: touch .claude/outcome.stop (in-flight step finishes; exit 3).
# Session/usage limit: exit 4 (forever wrapper sleeps and retries).
#
# Do NOT run this and autonomy-loop.sh in the same checkout at the same time —
# both stash/sync/commit/push the working tree.
#
# Usage:
#   bash .claude/scripts/outcome-loop.sh
#   MAX_ITERATIONS=50 MODEL_PLAN=opus MODEL_IMPL=sonnet bash .claude/scripts/outcome-loop.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

STOP_FLAG="${STOP_FLAG:-${SCRIPT_DIR}/../outcome.stop}"
LOG_DIR="${LOG_DIR:-${HOME}/.claude/logs/outcome}"
mkdir -p "${LOG_DIR}"

MAX_ITERATIONS="${MAX_ITERATIONS:-25}"
PERMISSION_MODE="${PERMISSION_MODE:-bypassPermissions}"
MODEL_PLAN="${MODEL_PLAN:-opus}"
MODEL_IMPL="${MODEL_IMPL:-sonnet}"
ITER_COST_WARN="${ITER_COST_WARN:-15}"

# Same build-parallelism + per-iteration memory hardening as the autonomy loop
# (see its header comments and docs/handoffs/2026-06-21-autonomy-loop-ooms.md).
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-6}"
ITER_MEMORY_MAX="${ITER_MEMORY_MAX:-32G}"
ITER_MEMORY_HIGH="${ITER_MEMORY_HIGH:-28G}"

S_PLAN_READY="<<PLAN_READY>>"
S_PHASE_COMPLETE="<<PHASE_COMPLETE>>"
S_PHASE_BLOCKED="<<PHASE_BLOCKED>>"
S_OUTCOME_COMPLETE="<<OUTCOME_COMPLETE>>"
S_OUTCOME_BLOCKED="<<OUTCOME_BLOCKED>>"

cd "${REPO_ROOT}"

outcome_dir() {
  # .claude/outcome-backlog holds ordered outcome directories, one per
  # non-comment line. Return the first whose outcome.md **Status:** is
  # neither `done` nor `blocked`.
  local d status
  while IFS= read -r d; do
    [ -z "${d}" ] && continue
    [ -f "${d}/outcome.md" ] || continue
    status="$(grep -m1 '^\*\*Status:\*\*' "${d}/outcome.md" | sed -E 's/^\*\*Status:\*\*[[:space:]]*//')"
    case "${status}" in
      done|blocked) continue ;;
      *) printf '%s' "${d}"; return 0 ;;
    esac
  done < <(grep -v '^\s*#' "${SCRIPT_DIR}/../outcome-backlog" 2>/dev/null)
  return 1
}

# Echo "<step> <phase#> <title>" for the first workable phase row, where
# <step> is `plan` (row pending) or `implement` (row planned). Empty output
# means no workable row (outcome exhausted -> dispatch a PLAN step to judge
# success criteria and emit a terminal sentinel).
next_step() {
  local dir="$1"
  awk -F'|' '
    /^\|/ {
      n=$2; t=$3; s=$4
      gsub(/^[ ]+|[ ]+$/, "", n); gsub(/^[ ]+|[ ]+$/, "", t); gsub(/^[ `]+|[ `]+$/, "", s)
      if (n !~ /^[0-9]+[a-z]?$/) next
      if (s == "pending")  { print "plan " n " " t; exit }
      if (s == "planned")  { print "implement " n " " t; exit }
    }' "${dir}/outcome.md"
}

echo "===== Outcome loop starting ====="
echo "Repo:            ${REPO_ROOT}"
ACTIVE="$(outcome_dir)" || {
  echo "===== no workable outcome in .claude/outcome-backlog (all done/blocked, or backlog empty/missing) ====="
  if grep -lF '**Status:** blocked' $(grep -v '^\s*#' "${SCRIPT_DIR}/../outcome-backlog" 2>/dev/null | sed 's#$#/outcome.md#') >/dev/null 2>&1; then
    exit 2
  fi
  exit 0
}
echo "Outcome:         ${ACTIVE}"
echo "Logs:            ${LOG_DIR}"
echo "Max iterations:  ${MAX_ITERATIONS}"
echo "Models:          plan=${MODEL_PLAN} implement=${MODEL_IMPL}"

# Per-iteration memory scope (degrades gracefully without systemd-run).
ITER_SCOPE_BASE=()
if command -v systemd-run >/dev/null 2>&1; then
  if systemd-run --user --scope --quiet --collect --unit="outcome-captest-$$" \
       -p MemoryHigh="${ITER_MEMORY_HIGH}" -p MemoryMax="${ITER_MEMORY_MAX}" \
       -p ManagedOOMPreference=avoid -- true >/dev/null 2>&1; then
    ITER_SCOPE_BASE=(systemd-run --user --scope --quiet --collect \
      -p MemoryHigh="${ITER_MEMORY_HIGH}" -p MemoryMax="${ITER_MEMORY_MAX}" \
      -p ManagedOOMPreference=avoid)
  elif systemd-run --user --scope --quiet --collect --unit="outcome-captest2-$$" \
         -p MemoryHigh="${ITER_MEMORY_HIGH}" -p MemoryMax="${ITER_MEMORY_MAX}" \
         -- true >/dev/null 2>&1; then
    ITER_SCOPE_BASE=(systemd-run --user --scope --quiet --collect \
      -p MemoryHigh="${ITER_MEMORY_HIGH}" -p MemoryMax="${ITER_MEMORY_MAX}")
  fi
fi
[ "${#ITER_SCOPE_BASE[@]}" -gt 0 ] \
  && echo "Iter mem scope:  MemoryMax=${ITER_MEMORY_MAX} MemoryHigh=${ITER_MEMORY_HIGH}" \
  || echo "Iter mem scope:  <systemd-run unavailable — uncapped>"
echo

iteration=0
exit_reason="unknown"
blocked_count=0

trap 'echo; echo "===== Interrupted by user ====="; exit 130' INT TERM

while [ "${iteration}" -lt "${MAX_ITERATIONS}" ]; do
  if [ -f "${STOP_FLAG}" ]; then
    rm -f "${STOP_FLAG}"
    echo "===== graceful stop requested — exiting before next step ====="
    exit_reason="stopped_by_flag"
    break
  fi

  iteration=$((iteration + 1))
  ts="$(date +%Y%m%dT%H%M%S)"
  log="${LOG_DIR}/iter-${ts}-$(printf '%02d' "${iteration}").log"

  # Clean tree + sync with origin/main (same discipline + rationale as the
  # autonomy loop: stat-refresh, stash leftovers, never auto-resolve conflicts).
  git update-index -q --refresh >/dev/null 2>&1 || true
  if [ -n "$(git status --porcelain)" ]; then
    echo "===== working tree dirty at iteration start — stashing leftover changes ====="
    git stash push --include-untracked \
      --message "outcome-loop auto-stash $(date -u +%Y%m%dT%H%M%SZ)" || true
  fi
  if git fetch origin main --quiet && git merge --no-edit origin/main; then
    git push --quiet || echo "(merge push deferred — step will push at end)"
  else
    git merge --abort 2>/dev/null || true
    echo "===== merge conflict with origin/main — pausing for human ====="
    exit_reason="merge_conflict_origin_main"
    break
  fi

  # Dispatch: which step, which model, which prompt. Backlog exhaustion
  # (every entry done/blocked) is a clean terminal state, not an error.
  ACTIVE="$(outcome_dir)" || {
    echo "===== no workable outcome left in .claude/outcome-backlog — all done/blocked ====="
    exit_reason="backlog_exhausted"
    break
  }
  step_info="$(next_step "${ACTIVE}")"
  if [ -n "${step_info}" ]; then
    step="${step_info%% *}"
    rest="${step_info#* }"
    phase_no="${rest%% *}"
    phase_title="${rest#* }"
    hint="WRAPPER PRE-SCAN HINT: the next workable row appears to be phase ${phase_no} (\`${phase_title}\`), status implies a ${step} step. Verify against the table in ${ACTIVE}/outcome.md; the table wins."
  else
    # Nothing workable: run a PLAN step so opus judges the success criteria
    # and emits <<OUTCOME_COMPLETE>> or <<OUTCOME_BLOCKED>>.
    step="plan"
    hint="WRAPPER PRE-SCAN HINT: no pending or planned rows remain in ${ACTIVE}/outcome.md — judge the success criteria and emit a terminal sentinel per the prompt's termination cases."
  fi
  if [ "${step}" = "plan" ]; then
    model="${MODEL_PLAN}"; prompt_file="${SCRIPT_DIR}/../outcome-plan-prompt.txt"
  else
    model="${MODEL_IMPL}"; prompt_file="${SCRIPT_DIR}/../outcome-implement-prompt.txt"
  fi

  echo "===== Iteration ${iteration}/${MAX_ITERATIONS} | ${ts} | step=${step} model=${model} ====="
  echo "Log: ${log}"
  echo "Pre-scan: ${hint}"

  iter_scope=()
  if [ "${#ITER_SCOPE_BASE[@]}" -gt 0 ]; then
    iter_scope=("${ITER_SCOPE_BASE[@]}" --unit="outcome-iter-${ts}-$(printf '%02d' "${iteration}")" --)
  fi

  "${iter_scope[@]}" claude --print \
    --permission-mode "${PERMISSION_MODE}" \
    --disallowedTools "ScheduleWakeup,Monitor" \
    --no-session-persistence \
    --model "${model}" \
    --output-format stream-json \
    --verbose \
    "$(cat "${prompt_file}")

${hint}" 2>&1 | tee "${log}" | "${SCRIPT_DIR}/format-stream-json.sh"
  rc="${PIPESTATUS[0]}"

  # Usage accounting (same shape as the autonomy loop, event tagged per step).
  USAGE_LOG="${REPO_ROOT}/.claude/usage-log.jsonl"
  jq -c --arg ts "${ts}" --arg step "${step}" --argjson iter "${iteration}" --argjson rc "${rc}" \
    'select(.type == "result") |
     {ts:$ts, event:("outcome-"+$step), iter:$iter, rc:$rc, session:.session_id,
      total_cost_usd:.total_cost_usd, duration_ms:.duration_ms, num_turns:.num_turns,
      input:.usage.input_tokens, output:.usage.output_tokens,
      cache_create:.usage.cache_creation_input_tokens, cache_read:.usage.cache_read_input_tokens}' \
    "${log}" >> "${USAGE_LOG}" 2>/dev/null || \
    echo "{\"ts\":\"${ts}\",\"event\":\"outcome-${step}\",\"iter\":${iteration},\"rc\":${rc},\"note\":\"unparseable-log\"}" >> "${USAGE_LOG}"

  iter_cost="$(jq -r 'select(.type=="result") | .total_cost_usd // 0' "${log}" 2>/dev/null | tail -1)"
  if [ -n "${iter_cost}" ] && awk -v c="${iter_cost}" -v w="${ITER_COST_WARN}" 'BEGIN{exit !(c>w)}'; then
    echo "===== WARNING: step cost \$${iter_cost} exceeded ITER_COST_WARN=\$${ITER_COST_WARN} — review ${log} ====="
  fi

  final_result="$(jq -Rr 'fromjson? | select(.type == "result") | .result // empty' "${log}" 2>/dev/null)"
  api_error_status="$(jq -Rr 'fromjson? | select(.type == "result") | .api_error_status // empty' "${log}" 2>/dev/null | tail -1)"

  # 429 / session-limit before the rc check (a limit hit makes claude exit
  # nonzero; it must classify as retry-later, not crash).
  if [ "${api_error_status}" = "429" ] \
     || printf '%s' "${final_result}" | grep -qiE 'session limit|usage limit'; then
    echo "===== session/usage limit hit — not a crash, retry later ====="
    exit_reason="session_limit"
    break
  fi
  if [ "${rc}" -ne 0 ]; then
    echo "===== claude exited with code ${rc} — pausing loop ====="
    exit_reason="claude_nonzero_${rc}"
    break
  fi
  if [ -z "${final_result}" ]; then
    echo "===== no final .result envelope in log — pausing loop ====="
    exit_reason="no_result_envelope"
    break
  fi

  if printf '%s' "${final_result}" | grep -qF "${S_OUTCOME_COMPLETE}"; then
    echo "===== ${S_OUTCOME_COMPLETE} — outcome achieved; advancing to next backlog entry ====="
    continue
  elif printf '%s' "${final_result}" | grep -qF "${S_OUTCOME_BLOCKED}"; then
    echo "===== ${S_OUTCOME_BLOCKED} — needs a human; advancing to next backlog entry ====="
    blocked_count=$((blocked_count + 1))
    continue
  elif printf '%s' "${final_result}" | grep -qF "${S_PHASE_BLOCKED}"; then
    blocked_count=$((blocked_count + 1))
    echo "===== ${S_PHASE_BLOCKED} — recorded; continuing to next row ====="
    continue
  elif printf '%s' "${final_result}" | grep -qF "${S_PLAN_READY}"; then
    echo "===== ${S_PLAN_READY} — next iteration implements ====="
    continue
  elif printf '%s' "${final_result}" | grep -qF "${S_PHASE_COMPLETE}"; then
    echo "===== ${S_PHASE_COMPLETE} — next iteration plans ====="
    continue
  else
    echo "===== no sentinel in final result — pausing loop (contract violation; see ${log}) ====="
    exit_reason="no_sentinel"
    break
  fi
done

if [ "${iteration}" -ge "${MAX_ITERATIONS}" ] && [ "${exit_reason}" = "unknown" ]; then
  exit_reason="max_iterations"
fi

echo
echo "===== Outcome loop exited: ${exit_reason} after ${iteration} iteration(s); blocked rows this run: ${blocked_count} ====="

case "${exit_reason}" in
  backlog_exhausted)
    [ "${blocked_count}" -gt 0 ] && exit 2
    exit 0
    ;;
  stopped_by_flag)    exit 3 ;;
  session_limit)      exit 4 ;;
  *)                  exit 1 ;;
esac
