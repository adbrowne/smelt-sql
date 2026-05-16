#!/usr/bin/env bash
#
# DEBUG version of autonomy-loop.sh.
#
# Purpose: diagnose the systemd-oomd kill that nukes the parent tmux
# scope (see journalctl --user — two kills on 2026-05-15 at 18:22 and
# 23:32, 46.3G and 42.0G peak memory respectively).
#
# Differences from autonomy-loop.sh:
#
#   1. Each iteration runs in its OWN user-scope cgroup, started by
#      systemd-run as a sibling of the tmux scope (not a child).
#      systemd-oomd kills the heaviest cgroup under pressure — by
#      isolating claude into its own scope, only that scope dies.
#      The wrapper + tmux + memory sampler survive and can record
#      what happened.
#
#   2. Uses --output-format stream-json + --verbose so each turn /
#      tool call lands in the log immediately (not a single envelope
#      at exit). The tail of the log reveals what claude was doing
#      when it was killed.
#
#   3. A background memory sampler runs OUTSIDE the iter scope and
#      records `free`, `ps --sort=-rss`, and `systemd-cgtop` every
#      SAMPLE_INTERVAL seconds to a separate .memory.log file.
#      Even if claude writes nothing, the memory log will show the
#      growth curve and the offending processes.
#
#   4. After each iteration, journalctl is grepped for the scope name;
#      any oom-kill / Failed-with-result record is appended to the
#      iter log so the post-mortem evidence is in one place.
#
#   5. Cost / token logging to .claude/usage-log.jsonl is DROPPED.
#      Re-add the jq block from autonomy-loop.sh once the crash is
#      diagnosed and behavior is stable again.
#
# Usage:
#   bash .claude/scripts/autonomy-loop-debug.sh
#   MAX_ITERATIONS=3 SAMPLE_INTERVAL=5 bash .claude/scripts/autonomy-loop-debug.sh
#
# Logs:
#   ${HOME}/.claude/logs/meta-language-loop/iter-<ts>-<n>.log
#   ${HOME}/.claude/logs/meta-language-loop/iter-<ts>-<n>.memory.log

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

LOG_DIR="${HOME}/.claude/logs/meta-language-loop"
mkdir -p "${LOG_DIR}"

MAX_ITERATIONS="${MAX_ITERATIONS:-25}"
PERMISSION_MODE="${PERMISSION_MODE:-bypassPermissions}"
MODEL="${MODEL:-opus}"
SAMPLE_INTERVAL="${SAMPLE_INTERVAL:-10}"

SENTINEL_PHASE="<<PHASE_COMPLETE>>"
SENTINEL_DONE="<<ALL_DONE>>"
SENTINEL_PAUSE="<<PAUSE_FOR_HUMAN>>"

cd "${REPO_ROOT}"

echo "===== Autonomy loop (DEBUG) starting ====="
echo "Repo:             ${REPO_ROOT}"
echo "Logs:             ${LOG_DIR}"
echo "Max iterations:   ${MAX_ITERATIONS}"
echo "Permission mode:  ${PERMISSION_MODE}"
echo "Model:            ${MODEL}"
echo "Sample interval:  ${SAMPLE_INTERVAL}s"
echo "Sentinels:        ${SENTINEL_PHASE} | ${SENTINEL_DONE} | ${SENTINEL_PAUSE}"
echo

# Background sampler — records system + per-process memory snapshots.
# Runs in the wrapper's cgroup (the tmux scope), NOT in the iter scope,
# so it survives if oomd kills the iter scope.
sample_memory() {
    local out="$1"
    local iter_scope="$2"
    while true; do
        {
            echo "===== $(date -Is) ====="
            echo "--- free -m ---"
            free -m
            echo
            echo "--- top 20 processes by RSS ---"
            ps -eo rss=,pid=,user=,comm=,args= --sort=-rss 2>/dev/null | head -20
            echo
            echo "--- systemd-cgtop user (one shot) ---"
            systemd-cgtop --user -1 -n 1 -b -m 2>/dev/null | head -30
            echo
            echo "--- iter scope memory.current / memory.peak ---"
            systemctl --user show "${iter_scope}" \
                -p MemoryCurrent -p MemoryPeak -p TasksCurrent 2>/dev/null \
                | grep -v '=$' || true
            echo
        } >> "${out}" 2>&1
        sleep "${SAMPLE_INTERVAL}"
    done
}

iteration=0
exit_reason="unknown"

trap 'echo; echo "===== Interrupted by user ====="; kill "${sampler_pid:-0}" 2>/dev/null; exit 130' INT TERM

while [ "${iteration}" -lt "${MAX_ITERATIONS}" ]; do
    iteration=$((iteration + 1))
    ts="$(date +%Y%m%dT%H%M%S)"
    n="$(printf '%02d' "${iteration}")"
    log="${LOG_DIR}/iter-${ts}-${n}.log"
    memlog="${LOG_DIR}/iter-${ts}-${n}.memory.log"
    scope="autonomy-iter-${ts}-${n}.scope"
    since="$(date '+%Y-%m-%d %H:%M:%S')"

    echo "===== Iteration ${iteration} of ${MAX_ITERATIONS} | ${ts} ====="
    echo "Iter log:   ${log}"
    echo "Memory log: ${memlog}"
    echo "Scope:      ${scope}"
    echo

    # Start the sampler before launching claude so we capture the
    # baseline + ramp. The sampler is in the wrapper's cgroup, not
    # the iter scope — it will not be killed when the iter scope dies.
    sample_memory "${memlog}" "${scope}" &
    sampler_pid=$!

    # Run claude in its own transient user scope. Notes:
    #
    #   --user --scope    creates a scope under the user's manager as
    #                     a sibling of the tmux scope, so oomd targets
    #                     it instead of tmux.
    #   --unit=...        predictable name so we can grep journalctl
    #                     afterwards for oom-kill records.
    #   --collect         drop the transient unit after exit so we don't
    #                     accumulate dead units across iterations.
    #   --quiet           suppress systemd-run's own "Running as unit:"
    #                     noise. The interesting output is claude's.
    #
    # NB: --pipe/--pty are NOT valid with --scope (scope runs in the
    # caller's session and inherits stdio directly). Don't add them.
    #
    # We DELIBERATELY do not set MemoryMax / MemoryHigh — the goal is
    # to observe the same growth pattern that crashed the parent
    # scope, not to clip it. Add `-p MemoryMax=24G` if you want to
    # bound iterations once the cause is understood.
    #
    # tee preserves live visibility in the tmux pane while still
    # writing every line to disk. Since stream-json flushes per-line
    # this should be reasonably real-time.
    systemd-run \
        --user \
        --scope \
        --unit="${scope}" \
        --collect \
        --quiet \
        -- \
        claude --print \
            --permission-mode "${PERMISSION_MODE}" \
            --no-session-persistence \
            --model "${MODEL}" \
            --verbose \
            --output-format stream-json \
            "continue" 2>&1 | tee "${log}"
    rc="${PIPESTATUS[0]}"

    # Stop the sampler.
    kill "${sampler_pid}" 2>/dev/null
    wait "${sampler_pid}" 2>/dev/null
    sampler_pid=""

    # Pull any oom-kill / Failed records for this scope into the iter
    # log so post-mortem is one-stop. Use --since the iteration start
    # to bound the journal scan.
    {
        echo
        echo "===== systemd journal for ${scope} ====="
        journalctl --user --since "${since}" --no-pager 2>&1 \
            | grep -F "${scope}" || echo "(no journal entries matched)"
    } >> "${log}"

    echo
    echo "claude exit code: ${rc}"

    if [ "${rc}" -ne 0 ]; then
        echo "===== claude exited with code ${rc} — pausing loop ====="
        echo "  Iter log:   ${log}"
        echo "  Memory log: ${memlog}"
        exit_reason="claude_nonzero_${rc}"
        break
    fi

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
        echo "  Iter log:   ${log}"
        echo "  Memory log: ${memlog}"
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
