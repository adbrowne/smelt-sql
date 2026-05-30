#!/usr/bin/env bash
#
# Autonomy loop for the active multi-session plan.
#
# Each iteration runs `claude -p "continue"` headlessly with a fresh
# context. Claude reads the meta-plan and the in-repo phase status
# table, finds the next pending phase, executes it (spec increment if
# applicable + /smelt:plan + /smelt:implement + expert reviews +
# verification gate + commit + push), and emits a sentinel string in
# its final output. The wrapper greps for the sentinel and decides
# what to do next.
#
# The orchestration logic in this script is plan-agnostic. The active
# plan is selected by the fresh-context discovery rules in
# `.claude/active-plan`.
#
# Currently active:
#   in-repo plan: docs/plans/20260530-feature-sweep.md
#   meta-plan:    ~/.claude/plans/we-now-have-a-idempotent-torvalds.md
#
# NOTE: this drives a bug-hunting sweep, not a feature plan. The default
# PROMPT below assumes /smelt:plan+/smelt:implement; for the sweep, run with
# the bug-hunt iteration prompt instead:
#   PROMPT="$(cat .claude/bug-hunt-prompt.txt)" bash .claude/scripts/autonomy-loop.sh
#
# When swapping plans, update .claude/active-plan, the two comment
# lines above, and the LOG_DIR below.
#
# Sentinels (Claude must emit one and only one of these per iteration):
#
#   <<PHASE_COMPLETE>>     — phase committed and pushed cleanly. Loop
#                            again with fresh context for the next phase.
#   <<ALL_DONE>>           — all phases done, verification green.
#                            Exit the loop with success.
#   <<PAUSE_FOR_HUMAN>>    — any stop-the-line condition fired
#                            (see meta-plan stop-the-line section).
#                            Exit the loop and surface to the user.
#
# Usage:
#   bash .claude/scripts/autonomy-loop.sh                # default: 25 iterations max
#   MAX_ITERATIONS=50 bash .claude/scripts/autonomy-loop.sh
#   PERMISSION_MODE=acceptEdits bash .claude/scripts/autonomy-loop.sh   # override
#
# Stop the loop manually: Ctrl-C. The current Claude iteration will
# finish naturally; the next iteration will not start.
#
# Logs: ${HOME}/.claude/logs/feature-sweep/iter-<ts>-<n>.log
#       ${HOME}/.claude/logs/feature-sweep/iter-<ts>-<n>.memory.log
# Each iteration's full stdout+stderr is captured for post-hoc review,
# alongside a periodic memory snapshot (free, top RSS processes, parent
# cgroup memory.current / memory.peak) so the next systemd-oomd kill has
# evidence to chew on.

set -uo pipefail

# Locate the repo root: this script lives at .claude/scripts/, repo is two up.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

LOG_DIR="${HOME}/.claude/logs/feature-sweep"
mkdir -p "${LOG_DIR}"

# Tunables (env vars override).
MAX_ITERATIONS="${MAX_ITERATIONS:-25}"
PERMISSION_MODE="${PERMISSION_MODE:-bypassPermissions}"
MODEL="${MODEL:-opus}"
SAMPLE_INTERVAL="${SAMPLE_INTERVAL:-10}"

SENTINEL_PHASE="<<PHASE_COMPLETE>>"
SENTINEL_DONE="<<ALL_DONE>>"
SENTINEL_PAUSE="<<PAUSE_FOR_HUMAN>>"

# Prompt sent to each iteration. Anchors the agent on the plan files and the
# sentinel emission contract — the wrapper greps the final .result for exactly
# one sentinel and pauses without one. Override with $PROMPT.
PROMPT="${PROMPT:-Resume the active autonomy loop with fresh context.

1. Read .claude/active-plan to find the in-repo plan and meta-plan paths for the currently active multi-session plan.
2. Read the meta-plan (across-session source of truth: sentinel emission contract, expert dispatch, stop-the-line conditions).
3. Read the in-repo plan (committed phase status table).
4. Find the next \`pending\` row in the status table. Execute that phase end-to-end: generate the per-phase plan via /smelt:plan if it does not exist yet, then /smelt:implement, expert review loop, verification gate, update the status table row, commit, push.

CRITICAL — sentinel emission contract: your final user-facing message MUST contain exactly one of ${SENTINEL_PHASE}, ${SENTINEL_DONE}, or ${SENTINEL_PAUSE}, per meta-plan §\"Sentinel emission contract\". The wrapper script greps the final .result field for these; without one the loop pauses and you stall progress. If you emit ${SENTINEL_PAUSE}, put a one-line reason on the line above.}"

cd "${REPO_ROOT}"

# Find our cgroup (the tmux-spawn scope, when launched from tmux). systemd-oomd
# kills a whole scope when it fires, so memory.current / memory.peak on this
# path is the number to watch. Best-effort — empty if /proc/self/cgroup is
# unreadable or we're not in a cgroup v2 hierarchy, in which case the sampler
# falls back to system-wide stats only.
SELF_CGROUP=""
if [ -r /proc/self/cgroup ]; then
    SELF_CGROUP="$(awk -F: '$1=="0"{print $3; exit}' /proc/self/cgroup 2>/dev/null || true)"
fi

# Background memory sampler. Writes a timestamped snapshot of system memory,
# top RSS processes, and the parent cgroup's memory counters every
# ${SAMPLE_INTERVAL}s. Without this we have no evidence whether rustc, cargo,
# claude, or something else was the heaviest tenant when oomd next pulls the
# trigger.
sample_memory() {
    local out="$1"
    while true; do
        {
            echo "===== $(date -Is) ====="
            echo "--- free -m ---"
            free -m
            echo
            echo "--- top 20 processes by RSS (kB) ---"
            ps -eo rss=,pid=,user=,comm=,args= --sort=-rss 2>/dev/null | head -20
            if [ -n "${SELF_CGROUP}" ] && [ -d "/sys/fs/cgroup${SELF_CGROUP}" ]; then
                echo
                echo "--- cgroup ${SELF_CGROUP} ---"
                for f in memory.current memory.peak memory.swap.current pids.current; do
                    if [ -r "/sys/fs/cgroup${SELF_CGROUP}/${f}" ]; then
                        printf '  %-22s %s\n' "${f}" "$(cat "/sys/fs/cgroup${SELF_CGROUP}/${f}")"
                    fi
                done
            fi
            echo
        } >> "${out}" 2>&1
        sleep "${SAMPLE_INTERVAL}"
    done
}

echo "===== Autonomy loop starting ====="
echo "Repo:            ${REPO_ROOT}"
echo "Logs:            ${LOG_DIR}"
echo "Max iterations:  ${MAX_ITERATIONS}"
echo "Permission mode: ${PERMISSION_MODE}"
echo "Model:           ${MODEL}"
echo "Sample interval: ${SAMPLE_INTERVAL}s"
echo "Sentinels:       ${SENTINEL_PHASE} | ${SENTINEL_DONE} | ${SENTINEL_PAUSE}"
echo "Cgroup watched:  ${SELF_CGROUP:-<unknown — sampler will skip cgroup stats>}"
echo

iteration=0
exit_reason="unknown"
sampler_pid=""

trap 'echo; echo "===== Interrupted by user ====="; [ -n "${sampler_pid}" ] && kill "${sampler_pid}" 2>/dev/null; exit 130' INT TERM

while [ "${iteration}" -lt "${MAX_ITERATIONS}" ]; do
  iteration=$((iteration + 1))
  ts="$(date +%Y%m%dT%H%M%S)"
  log="${LOG_DIR}/iter-${ts}-$(printf '%02d' "${iteration}").log"
  memlog="${LOG_DIR}/iter-${ts}-$(printf '%02d' "${iteration}").memory.log"

  echo "===== Iteration ${iteration} of ${MAX_ITERATIONS} | ${ts} ====="
  echo "Log:        ${log}"
  echo "Memory log: ${memlog}"
  echo

  # Reset the cgroup peak counter (cgroup v2 ≥ 6.5) so memory.peak in this
  # iter's samples reflects only this iteration, not the high-water mark
  # carried over from prior iterations. Silently no-ops on older kernels.
  if [ -n "${SELF_CGROUP}" ] && [ -w "/sys/fs/cgroup${SELF_CGROUP}/memory.peak" ]; then
      echo 0 > "/sys/fs/cgroup${SELF_CGROUP}/memory.peak" 2>/dev/null || true
  fi

  # Start the sampler before claude so we capture the baseline + ramp. It
  # runs in the same cgroup as the wrapper; if oomd kills the cgroup the
  # sampler dies too, but every snapshot up to that moment is already on
  # disk and survives.
  sample_memory "${memlog}" &
  sampler_pid=$!

  # --print:                         headless, single response, exit
  # --permission-mode:               unattended; bypass by default (autonomy loop)
  # --no-session-persistence:        each iteration is genuinely fresh
  # --model:                         orchestrator on opus per meta-plan
  # --output-format json:            single-envelope result with .usage + .total_cost_usd,
  #                                  so we can record per-iteration spend below.
  #                                  Sentinel grepped from .result via jq below.
  claude --print \
    --permission-mode "${PERMISSION_MODE}" \
    --no-session-persistence \
    --model "${MODEL}" \
    --output-format json \
    "${PROMPT}" 2>&1 | tee "${log}"
  rc="${PIPESTATUS[0]}"

  # Always stop the sampler before any break/continue below.
  kill "${sampler_pid}" 2>/dev/null || true
  wait "${sampler_pid}" 2>/dev/null || true
  sampler_pid=""

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

  # Sentinels must appear in the agent's final user-facing message (.result),
  # not anywhere in the streamed log. Plan files document the sentinel strings,
  # so reading them produces tool_use_result payloads that contain the literals
  # verbatim — grepping the whole log false-positives. Extract just .result.
  final_result="$(jq -r 'select(.type == "result") | .result // empty' "${log}" 2>/dev/null)"

  if [ -z "${final_result}" ]; then
    echo "===== Could not extract final .result from log — pausing loop ====="
    echo "(Expected a JSON envelope with .type == \"result\". Check the log: ${log})"
    exit_reason="no_result_envelope"
    break
  fi

  if printf '%s' "${final_result}" | grep -qF "${SENTINEL_DONE}"; then
    echo "===== ${SENTINEL_DONE} detected in final result — loop complete ====="
    exit_reason="all_done"
    break
  elif printf '%s' "${final_result}" | grep -qF "${SENTINEL_PAUSE}"; then
    echo "===== ${SENTINEL_PAUSE} detected in final result — surface to user ====="
    exit_reason="paused"
    break
  elif printf '%s' "${final_result}" | grep -qF "${SENTINEL_PHASE}"; then
    echo "===== ${SENTINEL_PHASE} detected in final result — starting next iteration ====="
    continue
  else
    echo "===== No sentinel in final result — pausing loop ====="
    echo "(The agent's final message did not contain a sentinel. Check the log: ${log})"
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
