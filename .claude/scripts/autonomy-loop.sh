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
# UNIFIED PRIORITY MODE (default — see .claude/sweep-loop-prompt.txt).
# There is no longer a manual probe-vs-remediation mode switch. Each
# iteration the agent applies a fixed priority:
#
#   PRIORITY 1 — READY REMEDIATION. Read the master plan's
#     "## Spawned sub-plans (remediation)" registry. A sub-plan is READY
#     when its registry Status is not `done` AND it has a `pending` phase.
#     If any is ready, execute its next pending phase (the sub-plan's own
#     self-contained per-phase routine) — remediation always runs before
#     scanning resumes.
#   PRIORITY 2 — SCAN. Only when NO sub-plan is ready, run the next
#     `pending` probe row in the master's Progress-tracking table via the
#     probe routine in .claude/bug-hunt-prompt.txt.
#   NOTHING READY — no ready sub-plan AND no pending probe row: emit
#     <<MASTER_EXHAUSTED>> and surface to the human (review needs-review
#     ledger items / scaffold the next sub-plan).
#
# This implements the intended workflow: run the loop whenever tokens are
# available; review found bugs out of band and promote a cluster into a
# sub-plan (scaffold the plan + add a NOT-`done` row to the master registry
# table); the next run fixes that ready work FIRST, then resumes scanning.
# The old `active_subplan` pointer and the prompt-swapping dance are
# retired — the registry table is the single source of "what's ready", so a
# bare `bash .claude/scripts/autonomy-loop.sh` does the right thing.
#
# The generic two-level remediation prompt is preserved inline below as
# REMEDIATION_PROMPT, and the standalone probe/remediation prompt files
# (.claude/bug-hunt-prompt.txt, .claude/diag-parity-prompt.txt) remain for
# reference and for running a single mode in isolation via PROMPT=.
#
# Sentinels (Claude must emit one and only one of these per iteration):
#
#   <<PHASE_COMPLETE>>     — phase committed and pushed cleanly. Loop
#                            again with fresh context for the next phase.
#   <<PHASE_BLOCKED>>      — the phase hit a design decision, or a red
#                            pre-flight that is NOT the phase's own target.
#                            The agent records it (marks the row `blocked`
#                            + a one-line reason, appends a dated entry to
#                            the active plan's "## Blocked phases" section,
#                            commits + pushes) and the loop CONTINUES to the
#                            next pending phase. NEVER halts — blocks are
#                            recorded for later human review, not stop-the-line.
#   <<SUBPLAN_ADVANCED>>   — the active sub-plan has no `pending` phases left
#                            (all done or blocked). The agent rolled up to the
#                            master plan, advanced .claude/active-plan to an
#                            EXISTING sibling sub-plan that has pending work,
#                            and committed. The loop continues on the new
#                            active sub-plan.
#   <<MASTER_EXHAUSTED>>   — the active sub-plan is exhausted and there is no
#                            existing sibling sub-plan with pending work to
#                            advance to (the next cluster would need a NEW
#                            sub-plan scaffolded — a human gate, per the
#                            conservative roll-up rule). Exit and surface the
#                            master-level summary to the user.
#   <<ALL_DONE>>           — master backlog fully remediated, verification
#                            green. Exit the loop with success.
#
# Legacy: <<PAUSE_FOR_HUMAN>> is treated as <<PHASE_BLOCKED>> (record +
# continue) — there is no agent-level hard-stop. Only wrapper-level infra
# failures (merge conflict, dirty tree, claude crash, missing sentinel) halt.
#
# Usage:
#   bash .claude/scripts/autonomy-loop.sh                # default: 25 iterations max
#   MAX_ITERATIONS=50 bash .claude/scripts/autonomy-loop.sh
#   PERMISSION_MODE=acceptEdits bash .claude/scripts/autonomy-loop.sh   # override
#
# Stop the loop:
#   - Graceful (recommended): `bash .claude/scripts/stop-autonomy.sh` (or
#     `touch .claude/autonomy.stop`). The in-flight iteration finishes — it
#     commits + pushes as normal — and then the loop exits before the next
#     iteration starts (exit code 3). No work is lost. Under the forever
#     wrapper this also prevents a restart. The flag is consumed on stop.
#   - Immediate: Ctrl-C / kill. The current Claude iteration is interrupted,
#     so its in-progress (uncommitted) work is wasted.
#
# Logs: ${HOME}/.claude/logs/spec-impl/iter-<ts>-<n>.log
#       ${HOME}/.claude/logs/spec-impl/iter-<ts>-<n>.memory.log
# Each iteration's full stdout+stderr is captured for post-hoc review,
# alongside a periodic memory snapshot (free, top RSS processes, parent
# cgroup memory.current / memory.peak) so the next systemd-oomd kill has
# evidence to chew on.

set -uo pipefail

# Locate the repo root: this script lives at .claude/scripts/, repo is two up.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# Graceful-stop flag. Checked at the top of every iteration: if present, the
# loop finishes the current iteration (already committed + pushed by then) and
# exits with code 3 without starting another. Lives under .claude/, which is
# gitignored by `.claude/*` — so it is never committed, and the iteration-start
# auto-stash (`git stash --include-untracked`, which skips ignored files) and
# `git status --porcelain` both leave it untouched. Create it with
# stop-autonomy.sh; it is removed automatically when the loop acts on it.
STOP_FLAG="${STOP_FLAG:-${SCRIPT_DIR}/../autonomy.stop}"

# Currently active: the refresh-as-maintenance-plan programme — spec alignment
# (SA1–SA5) then implementation (MP1–MP16: surface cut, plan derivation,
# diagnostics/explain, ledger, targeted-write/fold cells, propagation), under
# the model-updates master (docs/plans/20260704-model-updates.md via
# .claude/active-plan).
LOG_DIR="${HOME}/.claude/logs/maintenance-plan"
mkdir -p "${LOG_DIR}"

# Tunables (env vars override).
MAX_ITERATIONS="${MAX_ITERATIONS:-25}"
PERMISSION_MODE="${PERMISSION_MODE:-bypassPermissions}"
MODEL="${MODEL:-sonnet}"
SAMPLE_INTERVAL="${SAMPLE_INTERVAL:-10}"

# Cap cargo build parallelism. The host has 32 cores, so an unbounded
# `cargo test` fans out to 32 concurrent rustc + multiple rust-lld link jobs,
# each linking a statically-linked-DuckDB test binary. Those link-time RSS
# spikes drove the cgroup's memory *pressure* (PSI) over systemd-oomd's limit
# and got the whole tmux scope killed on 2026-05-31 (137 procs, oom-kill) even
# though absolute free RAM was ~50 GB. Capping parallel jobs flattens the
# spikes. Exported so every cargo invocation in the iteration inherits it.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-6}"

# Per-iteration memory isolation (infra hardening; see
# docs/handoffs/2026-06-21-autonomy-loop-ooms.md). Each iteration's `claude`
# (and the cargo/smelt builds it spawns) runs inside its own transient,
# memory-bounded systemd scope. Effect: a runaway iteration is killed *alone*
# by the kernel cgroup OOM-killer once it crosses ITER_MEMORY_MAX, BEFORE
# systemd-oomd reaps a whole tmux pane on memory *pressure* — which it picks by
# cgroup and can land on an unrelated session (the original collateral-kill
# bug). The supervisor (this script + forever-wrapper) stays OUTSIDE the scope,
# so it survives the kill and restarts the next iteration. MemoryHigh throttles
# (reclaim) before MemoryMax hard-kills, for a softer landing.
ITER_MEMORY_MAX="${ITER_MEMORY_MAX:-32G}"
ITER_MEMORY_HIGH="${ITER_MEMORY_HIGH:-28G}"

SENTINEL_PHASE="<<PHASE_COMPLETE>>"
SENTINEL_DONE="<<ALL_DONE>>"
SENTINEL_BLOCKED="<<PHASE_BLOCKED>>"
SENTINEL_ADVANCED="<<SUBPLAN_ADVANCED>>"
SENTINEL_MASTER_EXHAUSTED="<<MASTER_EXHAUSTED>>"
# Legacy sentinel — treated as SENTINEL_BLOCKED (record + continue, no halt).
SENTINEL_PAUSE="<<PAUSE_FOR_HUMAN>>"

# Generic two-level REMEDIATION prompt (active-plan driven). Preserved for when
# the loop swaps back into remediation mode — set PROMPT="$REMEDIATION_PROMPT"
# is not CLI-reachable, so to use it either re-point the default below or copy
# .claude/diag-parity-prompt.txt. Kept inline so the contract isn't lost.
REMEDIATION_PROMPT="Resume the active autonomy loop with fresh context.

STRUCTURE. The work is two-level. \`.claude/active-plan\` names a \`master_plan\` (the top-level feature backlog) and an \`active_subplan\` (the focused remediation plan the loop is currently working). You work the ACTIVE SUB-PLAN phase by phase; you only touch the master when the sub-plan is exhausted (see ROLL-UP).

1. Read \`.claude/active-plan\` for the \`master_plan\` and \`active_subplan\` paths.
2. Read the active sub-plan (committed phase status table + its per-phase routine, sentinel contract, and \"## Blocked phases\" section).
3. Find the next \`pending\` row in the status table (skip \`done\` and \`blocked\` rows). Execute that phase end-to-end per the per-phase routine in the sub-plan: spec increment if listed, /smelt:implement (red-green, implementer + reviewer), verification gates, update the row to \`done\`, commit, push. Emit ${SENTINEL_PHASE}.

RECORD-AND-CONTINUE (no hard-stop). If the phase hits a design decision not answered by the plan or spec, OR pre-flight \`cargo test\` is red on something that is NOT the acceptance target of this phase: do NOT halt. Instead (a) set the status-table row of this phase to \`blocked\` with a one-line reason, (b) append a dated entry to the \"## Blocked phases\" section of the sub-plan (phase id, the decision/reason, and the candidate options), (c) restore the tree to a clean committed state, (d) commit + push the table + blocked-log update, (e) emit ${SENTINEL_BLOCKED}. The next iteration will skip the blocked row and pick the next pending phase. NOTE: a red pre-flight that IS the acceptance target of this phase (the example/test the phase exists to make green) is EXPECTED — proceed, do not block on it.

ROLL-UP (only when the active sub-plan has NO \`pending\` rows left — all \`done\` or \`blocked\`). Consult the master plan and its ledger for the next un-remediated cluster. CONSERVATIVE RULE: you may only advance to an EXISTING sibling sub-plan that already has \`pending\` phases — if so, update \`active_subplan\` in \`.claude/active-plan\`, commit + push, and emit ${SENTINEL_ADVANCED}. You must NOT scaffold a brand-new sub-plan or author new specs/plans autonomously; if the next cluster has no existing sub-plan with pending work, emit ${SENTINEL_MASTER_EXHAUSTED} with a one-line summary of what remains (which clusters still need a human to scaffold a sub-plan, and how many phases are blocked). Emit ${SENTINEL_DONE} only if the master backlog is fully remediated and verification is green.

CRITICAL — sentinel emission contract: your final user-facing message MUST contain exactly one of ${SENTINEL_PHASE}, ${SENTINEL_BLOCKED}, ${SENTINEL_ADVANCED}, ${SENTINEL_MASTER_EXHAUSTED}, or ${SENTINEL_DONE}. The wrapper greps the final .result for these; without one the loop halts. Put any one-line reason on the line ABOVE the sentinel.}"

# Prompt sent to each iteration. DEFAULT = UNIFIED PRIORITY mode: read the
# sweep-loop prompt, which dispatches each iteration to ready remediation
# (master registry → first sub-plan with a pending phase) before falling back
# to the next pending probe row. It emits <<PHASE_COMPLETE>>/<<PHASE_BLOCKED>>/
# <<MASTER_EXHAUSTED>>/<<ALL_DONE>> (legacy <<PAUSE_FOR_HUMAN>> == BLOCKED).
# Override $PROMPT to force a single mode (e.g. PROMPT="$(cat
# .claude/bug-hunt-prompt.txt)" to scan only, or the diag-parity prompt to work
# one sub-plan only).
PROMPT="${PROMPT:-$(cat "${SCRIPT_DIR}/../sweep-loop-prompt.txt")}"

# Warn when a single iteration's spend crosses this (USD). Purely advisory —
# the $27 and $34 outlier iterations were reviewer/implementer thrash worth a
# human look at the time.
ITER_COST_WARN="${ITER_COST_WARN:-15}"

cd "${REPO_ROOT}"

# Pre-extract the next unit of work from the registry + status tables so the
# agent doesn't have to read the whole master plan (~45KB) + sub-plan just to
# find one row. Best-effort: emits a HINT line on stdout, or nothing (in which
# case the prompt is unchanged and the agent derives the phase itself). The
# hint is advisory — the tables stay the source of truth (the agent still
# updates them), so a stale/wrong hint costs nothing but the saved read.
phase_hint() {
  local master line sub status phase
  master="$(grep -E '^master_plan:' "${SCRIPT_DIR}/../active-plan" 2>/dev/null | tail -1 | awk '{print $2}')"
  [ -n "${master}" ] && [ -f "${master}" ] || return 0
  # Registry table rows under "## Spawned sub-plans", in order. Skip the
  # header/divider rows; each data row carries a docs/plans/ path and a
  # Status in its last cell.
  while IFS= read -r line; do
    sub="$(printf '%s' "${line}" | grep -oE 'docs/plans/[A-Za-z0-9._-]+\.md' | head -1)"
    [ -n "${sub}" ] && [ -f "${sub}" ] || continue
    status="$(printf '%s' "${line}" | awk -F'|' '{v=$(NF-1); gsub(/^[ `]+|[ `]+$/,"",v); print v}')"
    case "${status}" in done*|Done*) continue ;; esac
    # First `pending` row of the sub-plan's Progress table.
    phase="$(awk -F'|' '/^\|/ { p=$2; s=$3; gsub(/^ +| +$/,"",p); gsub(/^ +| +$/,"",s);
                               if (s=="pending") { print p; exit } }' "${sub}")"
    if [ -n "${phase}" ]; then
      printf 'WRAPPER PRE-SCAN HINT: the first READY sub-plan appears to be `%s` and its next `pending` phase appears to be `%s`. Verify against that sub-plan'"'"'s Progress table (one targeted read) instead of re-deriving from the full master plan; if the tables disagree with this hint, the tables win.' "${sub}" "${phase}"
      return 0
    fi
  done < <(awk '/^## Spawned sub-plans/{f=1; next} f && /^## /{exit} f' "${master}" \
             | grep -E '^\|' | grep -vE '^\|[-: ]+\||Sub-plan')
  return 0
}

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

# Export the sampler function and the globals it reads so the background
# sampler can run under a distinct argv[0] via `bash -c` (see launch below)
# without re-defining its body. Otherwise the backgrounded shell function
# inherits this script's command line and shows up in `ps` as a second
# `bash …autonomy-loop.sh`, which looks like a duplicate loop.
export -f sample_memory
export SELF_CGROUP SAMPLE_INTERVAL

echo "===== Autonomy loop starting ====="
echo "Repo:            ${REPO_ROOT}"
echo "Logs:            ${LOG_DIR}"
echo "Max iterations:  ${MAX_ITERATIONS}"
echo "Permission mode: ${PERMISSION_MODE}"
echo "Model:           ${MODEL}"
echo "Sample interval: ${SAMPLE_INTERVAL}s"
echo "Continue:        ${SENTINEL_PHASE} | ${SENTINEL_BLOCKED} | ${SENTINEL_ADVANCED}"
echo "Halt:            ${SENTINEL_DONE} | ${SENTINEL_MASTER_EXHAUSTED} (+ infra failures)"
echo "Cgroup watched:  ${SELF_CGROUP:-<unknown — sampler will skip cgroup stats>}"

# Assemble the per-iteration scope command once. Probe property support so an
# older systemd (no ManagedOOMPreference, etc.) degrades to caps-only, and a
# host without systemd-run degrades to running claude inline (uncapped).
ITER_SCOPE_BASE=()
if command -v systemd-run >/dev/null 2>&1; then
  if systemd-run --user --scope --quiet --collect --unit="autonomy-captest-$$" \
       -p MemoryHigh="${ITER_MEMORY_HIGH}" -p MemoryMax="${ITER_MEMORY_MAX}" \
       -p ManagedOOMPreference=avoid -- true >/dev/null 2>&1; then
    ITER_SCOPE_BASE=(systemd-run --user --scope --quiet --collect \
      -p MemoryHigh="${ITER_MEMORY_HIGH}" -p MemoryMax="${ITER_MEMORY_MAX}" \
      -p ManagedOOMPreference=avoid)
  elif systemd-run --user --scope --quiet --collect --unit="autonomy-captest2-$$" \
         -p MemoryHigh="${ITER_MEMORY_HIGH}" -p MemoryMax="${ITER_MEMORY_MAX}" \
         -- true >/dev/null 2>&1; then
    ITER_SCOPE_BASE=(systemd-run --user --scope --quiet --collect \
      -p MemoryHigh="${ITER_MEMORY_HIGH}" -p MemoryMax="${ITER_MEMORY_MAX}")
  fi
fi
if [ "${#ITER_SCOPE_BASE[@]}" -gt 0 ]; then
  echo "Iter mem scope:  MemoryMax=${ITER_MEMORY_MAX} MemoryHigh=${ITER_MEMORY_HIGH} (runaway dies alone)"
else
  echo "Iter mem scope:  <systemd-run unavailable — iterations run uncapped in this scope>"
fi
echo

iteration=0
exit_reason="unknown"
sampler_pid=""
blocked_count=0     # phases the agent recorded as `blocked` and continued past
advanced_count=0    # times the loop rolled up to a sibling sub-plan

trap 'echo; echo "===== Interrupted by user ====="; [ -n "${sampler_pid}" ] && kill "${sampler_pid}" 2>/dev/null; exit 130' INT TERM

while [ "${iteration}" -lt "${MAX_ITERATIONS}" ]; do
  # Graceful stop: honoured between iterations so the just-finished iteration's
  # committed work is preserved and nothing in flight is interrupted.
  if [ -f "${STOP_FLAG}" ]; then
    rm -f "${STOP_FLAG}"
    echo "===== graceful stop requested (${STOP_FLAG}) — finishing now; no further iterations ====="
    exit_reason="stopped_by_flag"
    break
  fi

  iteration=$((iteration + 1))
  ts="$(date +%Y%m%dT%H%M%S)"
  log="${LOG_DIR}/iter-${ts}-$(printf '%02d' "${iteration}").log"
  memlog="${LOG_DIR}/iter-${ts}-$(printf '%02d' "${iteration}").memory.log"

  echo "===== Iteration ${iteration} of ${MAX_ITERATIONS} | ${ts} ====="
  echo "Log:        ${log}"
  echo "Memory log: ${memlog}"
  echo

  # Keep the long-running sweep branch current with main before probing.
  # Placed here because the tree is clean at the top of an iteration (the
  # previous iteration committed + pushed; a relaunch starts clean). A clean
  # merge is committed (--no-edit) and pushed so origin stays current even if
  # this iteration later fails; a real CONFLICT is aborted and the loop pauses
  # for a human — never auto-resolved, since a wrong resolution would silently
  # corrupt every downstream phase. Runs before the sampler starts, so there is
  # no sampler to tear down on the pause paths below.
  # Drop pure stat-dirt first (mtime-only touches from a prior cargo/test run
  # that changed no bytes — the exact thing that stalled the loop 2026-06-06),
  # then stash any genuinely-leftover changes so a dirty tree never halts the
  # loop. Stash (not discard) keeps the changes recoverable via `git stash list`.
  git update-index -q --refresh >/dev/null 2>&1 || true
  if [ -n "$(git status --porcelain)" ]; then
    echo "===== working tree dirty at iteration start — stashing leftover changes ====="
    if git stash push --include-untracked \
         --message "autonomy-loop auto-stash $(date -u +%Y%m%dT%H%M%SZ) (dirty at iteration start)"; then
      echo "(leftover changes stashed; recover with 'git stash list' / 'git stash pop')"
    else
      echo "(nothing to stash after stat-refresh — proceeding)"
    fi
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
  # Relabel the backgrounded sampler so `ps` shows `smelt-mem-sampler`
  # instead of an identical `bash …autonomy-loop.sh` line (it's a forked
  # shell function, which otherwise inherits this script's command line and
  # looks like a second loop). exec -a sets argv[0]; the exported
  # sample_memory function (above) keeps the body in one place.
  ( exec -a smelt-mem-sampler bash -c 'sample_memory "$1"' smelt-mem-sampler "${memlog}" ) &
  sampler_pid=$!

  # --print:                         headless, single response, exit
  # --permission-mode:               unattended; bypass by default (autonomy loop)
  # --no-session-persistence:        each iteration is genuinely fresh
  # --model:                         control loop on ${MODEL} (default sonnet; override via MODEL=)
  # --output-format json:            single-envelope result with .usage + .total_cost_usd,
  #                                  so we can record per-iteration spend below.
  #                                  Sentinel grepped from .result via jq below.
  # Wrap claude in this iteration's memory-bounded scope (no-op prefix when
  # systemd-run is unavailable). systemd-run --scope propagates claude's exit
  # code, so PIPESTATUS[0] still reflects claude (or the OOM-kill that felled
  # the scope, which is then handled as a non-zero iteration below).
  iter_scope=()
  if [ "${#ITER_SCOPE_BASE[@]}" -gt 0 ]; then
    iter_scope=("${ITER_SCOPE_BASE[@]}" --unit="autonomy-iter-${ts}-$(printf '%02d' "${iteration}")" --)
  fi
  # Recomputed every iteration — the previous iteration flipped a table row.
  hint="$(phase_hint)"
  [ -n "${hint}" ] && echo "Pre-scan:   ${hint}"
  "${iter_scope[@]}" claude --print \
    --permission-mode "${PERMISSION_MODE}" \
    --no-session-persistence \
    --model "${MODEL}" \
    --output-format json \
    "${PROMPT}${hint:+

${hint}}" 2>&1 | tee "${log}"
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

  # Advisory cost check: a single phase costing >$ITER_COST_WARN usually means
  # implementer/reviewer thrash or a runaway context — worth a post-hoc look.
  iter_cost="$(jq -r 'select(.type=="result") | .total_cost_usd // 0' "${log}" 2>/dev/null | tail -1)"
  if [ -n "${iter_cost}" ] && awk -v c="${iter_cost}" -v w="${ITER_COST_WARN}" 'BEGIN{exit !(c>w)}'; then
    echo "===== WARNING: iteration cost \$${iter_cost} exceeded ITER_COST_WARN=\$${ITER_COST_WARN} — review ${log} ====="
  fi

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
  # -R/fromjson?: tolerate non-JSON lines interleaved into the --output-format
  # json stream (e.g. "Background tasks still running after 600s; terminating.",
  # emitted when the agent spawns async background work and returns). Without
  # this a single stray line makes jq abort before the result envelope, yielding
  # a spurious no_result_envelope pause instead of the real "no sentinel" path.
  final_result="$(jq -Rr 'fromjson? | select(.type == "result") | .result // empty' "${log}" 2>/dev/null)"
  api_error_status="$(jq -Rr 'fromjson? | select(.type == "result") | .api_error_status // empty' "${log}" 2>/dev/null | tail -1)"

  # Session/usage-limit hit (HTTP 429, or the "You've hit your session
  # limit · resets <time>" message the CLI prints instead of any sentinel).
  # This is NOT a crash — the account is simply out of budget until the
  # window resets. Classify it distinctly from no_sentinel/claude_nonzero so
  # the forever-wrapper never counts it toward its crash-loop (fast-fail)
  # guard; it must always retry, no matter how many times in a row it
  # recurs while waiting out the reset window.
  if [ "${api_error_status}" = "429" ] \
     || printf '%s' "${final_result}" | grep -qiE 'session limit|usage limit'; then
    echo "===== session/usage limit hit (429) — not a crash, will retry later ====="
    exit_reason="session_limit"
    break
  fi

  if [ -z "${final_result}" ]; then
    echo "===== Could not extract final .result from log — pausing loop ====="
    echo "(Expected a JSON envelope with .type == \"result\". Check the log: ${log})"
    exit_reason="no_result_envelope"
    break
  fi

  # Dispatch on the sentinel. Halting sentinels (DONE / MASTER_EXHAUSTED) break;
  # record-and-continue sentinels (BLOCKED / legacy PAUSE / ADVANCED / PHASE)
  # loop again. There is no agent-level hard-stop — a design block is recorded
  # by the agent (row → `blocked`, "## Blocked phases" entry, committed) and the
  # loop moves to the next pending phase. Only infra failures below halt.
  if printf '%s' "${final_result}" | grep -qF "${SENTINEL_DONE}"; then
    echo "===== ${SENTINEL_DONE} detected — master backlog remediated, loop complete ====="
    exit_reason="all_done"
    break
  elif printf '%s' "${final_result}" | grep -qF "${SENTINEL_MASTER_EXHAUSTED}"; then
    echo "===== ${SENTINEL_MASTER_EXHAUSTED} detected — no sibling sub-plan with pending work; surface to user ====="
    exit_reason="master_exhausted"
    break
  elif printf '%s' "${final_result}" | grep -qF "${SENTINEL_ADVANCED}"; then
    advanced_count=$((advanced_count + 1))
    echo "===== ${SENTINEL_ADVANCED} detected — advanced active sub-plan, continuing ====="
    continue
  elif printf '%s' "${final_result}" | grep -qF "${SENTINEL_BLOCKED}" \
       || printf '%s' "${final_result}" | grep -qF "${SENTINEL_PAUSE}"; then
    blocked_count=$((blocked_count + 1))
    echo "===== ${SENTINEL_BLOCKED} detected — block recorded in plan; continuing to next pending phase ====="
    continue
  elif printf '%s' "${final_result}" | grep -qF "${SENTINEL_PHASE}"; then
    echo "===== ${SENTINEL_PHASE} detected in final result — starting next iteration ====="
    continue
  else
    echo "===== No sentinel in final result — pausing loop (infra: agent did not follow contract) ====="
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
echo "Blocked phases recorded this run: ${blocked_count} (see the active plan's \"## Blocked phases\" section)"
echo "Sub-plan roll-ups this run:        ${advanced_count}"
echo "Logs in: ${LOG_DIR}"

# Exit codes: 0 = master backlog done; 2 = needs a human (master exhausted —
# next cluster needs a sub-plan scaffolded); 3 = graceful stop requested (do
# not restart); 4 = session/usage limit hit (always retry, never a fast-fail);
# 1 = infra failure / max-iter.
case "${exit_reason}" in
  all_done) exit 0 ;;
  master_exhausted) exit 2 ;;
  stopped_by_flag) exit 3 ;;
  session_limit) exit 4 ;;
  *) exit 1 ;;
esac
