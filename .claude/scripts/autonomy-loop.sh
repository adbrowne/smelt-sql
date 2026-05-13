#!/usr/bin/env bash
#
# Autonomy loop for the typed-meta-programming implementation.
#
# Each iteration runs `claude -p "continue"` headlessly with a fresh
# context. Claude reads the meta-plan and the in-repo phase status
# table, finds the next pending phase, executes it (spec increment +
# /smelt:plan + /smelt:implement + expert reviews + /smelt:validate +
# commit + push), and emits a sentinel string in its final output.
# The wrapper greps for the sentinel and decides what to do next.
#
# Sentinels (Claude must emit one and only one of these per iteration):
#
#   <<PHASE_COMPLETE>>     — phase committed and pushed cleanly. Loop
#                            again with fresh context for the next phase.
#   <<ALL_DONE>>           — all phases (A–G) done, verification green.
#                            Exit the loop with success.
#   <<PAUSE_FOR_HUMAN>>    — any stop-the-line condition fired
#                            (see meta-plan §7). Exit the loop and
#                            surface to the user.
#
# Usage:
#   bash .claude/scripts/autonomy-loop.sh                # default: 25 iterations max
#   MAX_ITERATIONS=50 bash .claude/scripts/autonomy-loop.sh
#   PERMISSION_MODE=acceptEdits bash .claude/scripts/autonomy-loop.sh   # override
#
# Stop the loop manually: Ctrl-C. The current Claude iteration will
# finish naturally; the next iteration will not start.
#
# Logs: ${HOME}/.claude/logs/meta-language-loop/iter-<ts>-<n>.log
# Each iteration's full stdout+stderr is captured for post-hoc review.

set -uo pipefail

# Locate the repo root: this script lives at .claude/scripts/, repo is two up.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

LOG_DIR="${HOME}/.claude/logs/meta-language-loop"
mkdir -p "${LOG_DIR}"

# Tunables (env vars override).
MAX_ITERATIONS="${MAX_ITERATIONS:-25}"
PERMISSION_MODE="${PERMISSION_MODE:-bypassPermissions}"
MODEL="${MODEL:-opus}"

SENTINEL_PHASE="<<PHASE_COMPLETE>>"
SENTINEL_DONE="<<ALL_DONE>>"
SENTINEL_PAUSE="<<PAUSE_FOR_HUMAN>>"

cd "${REPO_ROOT}"

echo "===== Autonomy loop starting ====="
echo "Repo:            ${REPO_ROOT}"
echo "Logs:            ${LOG_DIR}"
echo "Max iterations:  ${MAX_ITERATIONS}"
echo "Permission mode: ${PERMISSION_MODE}"
echo "Model:           ${MODEL}"
echo "Sentinels:       ${SENTINEL_PHASE} | ${SENTINEL_DONE} | ${SENTINEL_PAUSE}"
echo

iteration=0
exit_reason="unknown"

trap 'echo; echo "===== Interrupted by user ====="; exit 130' INT TERM

while [ "${iteration}" -lt "${MAX_ITERATIONS}" ]; do
  iteration=$((iteration + 1))
  ts="$(date +%Y%m%dT%H%M%S)"
  log="${LOG_DIR}/iter-${ts}-$(printf '%02d' "${iteration}").log"

  echo "===== Iteration ${iteration} of ${MAX_ITERATIONS} | ${ts} ====="
  echo "Log: ${log}"
  echo

  # --print:                         headless, single response, exit
  # --permission-mode:               unattended; bypass by default (autonomy loop)
  # --no-session-persistence:        each iteration is genuinely fresh
  # --model:                         orchestrator on opus per meta-plan
  # --output-format json:            single-envelope result with .usage + .total_cost_usd,
  #                                  so we can record per-iteration spend below.
  #                                  Sentinels still grep-able from the raw text.
  # Prompt is "continue" — Claude reads plan files to discover Phase X.
  claude --print \
    --permission-mode "${PERMISSION_MODE}" \
    --no-session-persistence \
    --model "${MODEL}" \
    --output-format json \
    "continue" 2>&1 | tee "${log}"
  rc="${PIPESTATUS[0]}"

  # Capture per-iteration token usage to .claude/usage-log.jsonl. The json
  # envelope from `claude --output-format json` includes .usage and
  # .total_cost_usd; if the log isn't valid json (e.g. claude crashed mid-output)
  # this is a no-op.
  USAGE_LOG="${REPO_ROOT}/.claude/usage-log.jsonl"
  mkdir -p "$(dirname "${USAGE_LOG}")"
  jq -c \
    --arg ts "${ts}" \
    --argjson iter "${iteration}" \
    --argjson rc "${rc}" \
    '{
       ts: $ts,
       event: "headless-iter",
       iter: $iter,
       rc: $rc,
       session: .session_id,
       total_cost_usd: .total_cost_usd,
       duration_ms: .duration_ms,
       num_turns: .num_turns,
       input: .usage.input_tokens,
       output: .usage.output_tokens,
       cache_create: .usage.cache_creation_input_tokens,
       cache_read: .usage.cache_read_input_tokens
     }' "${log}" >> "${USAGE_LOG}" 2>/dev/null || \
    echo "{\"ts\":\"${ts}\",\"event\":\"headless-iter\",\"iter\":${iteration},\"rc\":${rc},\"note\":\"unparseable-log\"}" >> "${USAGE_LOG}"

  echo

  if [ "${rc}" -ne 0 ]; then
    echo "===== claude exited with code ${rc} — pausing loop ====="
    exit_reason="claude_nonzero_${rc}"
    break
  fi

  # Inspect the log for sentinels (last write wins; check exact precedence).
  if grep -qF "${SENTINEL_DONE}" "${log}"; then
    echo "===== ${SENTINEL_DONE} detected — loop complete ====="
    exit_reason="all_done"
    break
  elif grep -qF "${SENTINEL_PAUSE}" "${log}"; then
    echo "===== ${SENTINEL_PAUSE} detected — surface to user ====="
    exit_reason="paused"
    break
  elif grep -qF "${SENTINEL_PHASE}" "${log}"; then
    echo "===== ${SENTINEL_PHASE} detected — starting next iteration ====="
    continue
  else
    echo "===== No sentinel detected — pausing loop ====="
    echo "(This is unexpected. Check the log: ${log})"
    exit_reason="no_sentinel"
    break
  fi
done

if [ "${iteration}" -ge "${MAX_ITERATIONS}" ] && [ "${exit_reason}" = "unknown" ]; then
  exit_reason="max_iterations"
  echo "===== Reached MAX_ITERATIONS=${MAX_ITERATIONS} — pausing loop ====="
fi

echo
echo "===== Loop exited: ${exit_reason} after ${iteration} iteration(s) ====="
echo "Logs in: ${LOG_DIR}"

case "${exit_reason}" in
  all_done) exit 0 ;;
  paused) exit 2 ;;
  *) exit 1 ;;
esac
