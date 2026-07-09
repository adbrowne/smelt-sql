#!/usr/bin/env bash
#
# Property-discovery research loop.
#
# Each iteration runs `claude -p` headlessly with a fresh context, works ONE
# catalog cell (docs/research/property-discovery/catalog.jsonl) end-to-end —
# author/extend a proptest, run it, record a verdict in the ledger, commit —
# and emits a sentinel the wrapper greps to decide what to do next.
#
# This is the RESEARCH sibling of autonomy-loop.sh. It is single-catalog (no
# master/sub-plan roll-up) and its sentinels are:
#
#   <<PROBE_COMPLETE>>    — cell resolved + committed. Loop again (fresh context).
#   <<PROBE_BLOCKED>>     — design fork / missing infra / unrelated red. Recorded
#                           in the cell + plan; loop CONTINUES to the next cell.
#   <<CATALOG_EXHAUSTED>> — no `pending` cell remains. Stop (exit 2); a human
#                           seeds the next tranche.
#
# The design + plan + per-cell routine live in:
#   docs/research/20260705-property-discovery-loop.md   (design)
#   docs/plans/20260705-property-discovery-loop.md       (plan)
#   .claude/property-loop-prompt.txt                     (iteration prompt)
#
# Usage:
#   DUCKDB_LIB_DIR=/usr/local/lib LD_LIBRARY_PATH=/usr/local/lib \
#     bash .claude/scripts/property-loop.sh                 # 25 iterations max
#   MAX_ITERATIONS=50 bash .claude/scripts/property-loop.sh
#
# Graceful stop: `touch .claude/property-loop.stop` (finishes the in-flight
# iteration — already committed — then exits 3 without starting another).
#
# Logs: ${HOME}/.claude/logs/property-discovery/iter-<ts>-<n>.log
#
# IMPORTANT: never run this concurrently with autonomy-loop.sh — both drive
# worktree-incremental and would race / auto-stash-clobber each other. Pause the
# fundamentals loop for the duration of this research (design §6).

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

STOP_FLAG="${STOP_FLAG:-${SCRIPT_DIR}/../property-loop.stop}"
LOG_DIR="${HOME}/.claude/logs/property-discovery"
mkdir -p "${LOG_DIR}"

MAX_ITERATIONS="${MAX_ITERATIONS:-25}"
PERMISSION_MODE="${PERMISSION_MODE:-bypassPermissions}"
MODEL="${MODEL:-sonnet}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-6}"
ITER_MEMORY_MAX="${ITER_MEMORY_MAX:-32G}"
ITER_MEMORY_HIGH="${ITER_MEMORY_HIGH:-28G}"

SENTINEL_COMPLETE="<<PROBE_COMPLETE>>"
SENTINEL_BLOCKED="<<PROBE_BLOCKED>>"
SENTINEL_EXHAUSTED="<<CATALOG_EXHAUSTED>>"

PROMPT="${PROMPT:-$(cat "${SCRIPT_DIR}/../property-loop-prompt.txt")}"

cd "${REPO_ROOT}"

# Assemble the per-iteration memory-bounded scope once (degrade gracefully when
# systemd-run is unavailable / rejects a property — same pattern as the autonomy
# loop, so a runaway iteration is OOM-killed alone rather than felling the pane).
ITER_SCOPE_BASE=()
if command -v systemd-run >/dev/null 2>&1; then
  if systemd-run --user --scope --quiet --collect --unit="proploop-captest-$$" \
       -p MemoryHigh="${ITER_MEMORY_HIGH}" -p MemoryMax="${ITER_MEMORY_MAX}" \
       -p ManagedOOMPreference=avoid -- true >/dev/null 2>&1; then
    ITER_SCOPE_BASE=(systemd-run --user --scope --quiet --collect \
      -p MemoryHigh="${ITER_MEMORY_HIGH}" -p MemoryMax="${ITER_MEMORY_MAX}" \
      -p ManagedOOMPreference=avoid)
  elif systemd-run --user --scope --quiet --collect --unit="proploop-captest2-$$" \
         -p MemoryHigh="${ITER_MEMORY_HIGH}" -p MemoryMax="${ITER_MEMORY_MAX}" \
         -- true >/dev/null 2>&1; then
    ITER_SCOPE_BASE=(systemd-run --user --scope --quiet --collect \
      -p MemoryHigh="${ITER_MEMORY_HIGH}" -p MemoryMax="${ITER_MEMORY_MAX}")
  fi
fi

echo "===== Property-discovery loop starting ====="
echo "Repo:            ${REPO_ROOT}"
echo "Logs:            ${LOG_DIR}"
echo "Max iterations:  ${MAX_ITERATIONS}"
echo "Model:           ${MODEL}"
echo "Continue:        ${SENTINEL_COMPLETE} | ${SENTINEL_BLOCKED}"
echo "Halt:            ${SENTINEL_EXHAUSTED} (+ infra failures)"
if [ "${#ITER_SCOPE_BASE[@]}" -gt 0 ]; then
  echo "Iter mem scope:  MemoryMax=${ITER_MEMORY_MAX} MemoryHigh=${ITER_MEMORY_HIGH}"
else
  echo "Iter mem scope:  <systemd-run unavailable — iterations run uncapped>"
fi
if [ -z "${DUCKDB_LIB_DIR:-}" ]; then
  echo "WARNING: DUCKDB_LIB_DIR unset — DuckDB-backed cells will BLOCK. Export it before running."
fi
echo

iteration=0
exit_reason="unknown"
blocked_count=0

trap 'echo; echo "===== Interrupted ====="; exit 130' INT TERM

while [ "${iteration}" -lt "${MAX_ITERATIONS}" ]; do
  if [ -f "${STOP_FLAG}" ]; then
    rm -f "${STOP_FLAG}"
    echo "===== graceful stop requested (${STOP_FLAG}) — exiting ====="
    exit_reason="stopped_by_flag"
    break
  fi

  iteration=$((iteration + 1))
  ts="$(date +%Y%m%dT%H%M%S)"
  log="${LOG_DIR}/iter-${ts}-$(printf '%02d' "${iteration}").log"
  echo "===== Iteration ${iteration} of ${MAX_ITERATIONS} | ${ts} ====="
  echo "Log: ${log}"
  echo

  # Keep the branch current with origin/main at the top of the iteration (tree is
  # clean here — previous iteration committed). Drop stat-dirt, stash genuine
  # leftovers, then merge. A real conflict halts for a human (never auto-resolved).
  git update-index -q --refresh >/dev/null 2>&1 || true
  if [ -n "$(git status --porcelain)" ]; then
    echo "----- working tree dirty at iteration start — stashing leftovers -----"
    git stash push --include-untracked \
      --message "property-loop auto-stash $(date -u +%Y%m%dT%H%M%SZ)" || true
  fi
  echo "----- syncing branch with origin/main -----"
  if git fetch origin main --quiet && git merge --no-edit origin/main; then
    git push --quiet || echo "(merge push deferred — iteration will push at end)"
  else
    git merge --abort 2>/dev/null || true
    echo "===== merge conflict with origin/main — pausing for human ====="
    exit_reason="merge_conflict_origin_main"
    break
  fi
  echo

  iter_scope=()
  if [ "${#ITER_SCOPE_BASE[@]}" -gt 0 ]; then
    iter_scope=("${ITER_SCOPE_BASE[@]}" --unit="proploop-iter-${ts}-$(printf '%02d' "${iteration}")" --)
  fi
  "${iter_scope[@]}" claude --print \
    --permission-mode "${PERMISSION_MODE}" \
    --no-session-persistence \
    --model "${MODEL}" \
    --output-format json \
    "${PROMPT}" 2>&1 | tee "${log}"
  rc="${PIPESTATUS[0]}"

  # Record per-iteration usage.
  USAGE_LOG="${REPO_ROOT}/.claude/usage-log.jsonl"
  jq -c --arg ts "${ts}" --argjson iter "${iteration}" --argjson rc "${rc}" \
    '{ts:$ts,event:"property-iter",iter:$iter,rc:$rc,session:.session_id,
      total_cost_usd:.total_cost_usd,duration_ms:.duration_ms,num_turns:.num_turns,
      input:.usage.input_tokens,output:.usage.output_tokens,
      cache_read:.usage.cache_read_input_tokens}' "${log}" >> "${USAGE_LOG}" 2>/dev/null \
    || echo "{\"ts\":\"${ts}\",\"event\":\"property-iter\",\"iter\":${iteration},\"rc\":${rc},\"note\":\"unparseable-log\"}" >> "${USAGE_LOG}"
  echo

  if [ "${rc}" -ne 0 ]; then
    echo "===== claude exited with code ${rc} — pausing loop (wrapper will retry in 10m) ====="
    exit_reason="claude_nonzero_${rc}"
    break
  fi

  final_result="$(jq -Rr 'fromjson? | select(.type == "result") | .result // empty' "${log}" 2>/dev/null)"
  if [ -z "${final_result}" ]; then
    echo "===== Could not extract final .result — pausing loop ====="
    exit_reason="no_result_envelope"
    break
  fi

  if printf '%s' "${final_result}" | grep -qF "${SENTINEL_EXHAUSTED}"; then
    echo "===== ${SENTINEL_EXHAUSTED} — catalog has no pending cell; surface to human ====="
    exit_reason="catalog_exhausted"
    break
  elif printf '%s' "${final_result}" | grep -qF "${SENTINEL_BLOCKED}"; then
    blocked_count=$((blocked_count + 1))
    echo "===== ${SENTINEL_BLOCKED} — recorded; continuing to next cell ====="
    continue
  elif printf '%s' "${final_result}" | grep -qF "${SENTINEL_COMPLETE}"; then
    echo "===== ${SENTINEL_COMPLETE} — cell resolved; next iteration ====="
    continue
  else
    echo "===== No sentinel in final result — pausing loop (contract violation) ====="
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
echo "Cells blocked this run: ${blocked_count}"
echo "Logs in: ${LOG_DIR}"

# Exit codes: 0 reserved; 2 = catalog exhausted (needs a human); 3 = graceful
# stop; 1 = infra failure / max-iter (the forever wrapper retries after 10m).
case "${exit_reason}" in
  catalog_exhausted) exit 2 ;;
  stopped_by_flag) exit 3 ;;
  *) exit 1 ;;
esac
