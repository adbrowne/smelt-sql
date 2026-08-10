# Outcome loop — operator guide

A second headless loop, parallel to the autonomy loop (`docs/autonomy_loop.md`),
for **outcome-driven** work. The observed failure mode of upfront plans is that
they freeze scope on day one and then defer work that is actually in line with
the original goal. The outcome loop inverts this: the committed artifact is an
**outcome** (goal + checkable success criteria + a phase list of one-liners),
and the detailed plan for each phase is written **just before that phase runs**,
by a planner step that has read the previous phase's summary.

## The artifacts

Each outcome is a directory under `docs/outcomes/<YYYYMMDD-name>/`:

```
outcome.md            # goal, success criteria, out-of-scope, phase table,
                      # decision log, blocked log. Concise — phases are ONE LINE.
phases/NN-plan.md     # written by the PLAN step at the start of phase NN
phases/NN-summary.md  # written by the IMPLEMENT step at the end of phase NN
```

`.claude/outcome-backlog` is the **ordered backlog**: one outcome directory
per non-comment line. The loop works the first entry whose `outcome.md`
`**Status:**` is neither `done` nor `blocked`, and advances to the next when a
terminal sentinel marks it. Outcome statuses: `queued` → `active` → `done`
(or `blocked` — recorded, skipped, surfaced to a human later). Scaffold and
append a new outcome with `/smelt:outcome <name>`; reorder by editing the file.

Phase statuses: `pending` → `planned` → `done` (or `blocked`, recorded and
skipped, never stop-the-line).

## The state machine

Each iteration of `.claude/scripts/outcome-loop.sh` runs ONE step in a fresh
headless `claude --print`, chosen from the first non-`done`/non-`blocked` row
of the phase table:

- Row is `pending` → **PLAN step** (model: `MODEL_PLAN`, default **opus**).
  Reads `outcome.md` and the previous phase's `NN-summary.md` (only that — not
  the whole history), **reshapes the remaining phase list if the summary
  warrants it** (add/split/merge/reorder future rows; work serving the success
  criteria is never deferred out — only genuinely out-of-scope items leave,
  recorded under "Out of scope" with a line of rationale), writes a concise
  `phases/NN-plan.md`, flips the row to `planned`, commits, pushes. Emits
  `<<PLAN_READY>>`.
- Row is `planned` → **IMPLEMENT step** (model: `MODEL_IMPL`, default
  **sonnet**). Reads `outcome.md` + `phases/NN-plan.md`, executes red-green,
  runs `bash .claude/scripts/verify-phase.sh`, writes `phases/NN-summary.md`
  (≤40 lines: what shipped, decisions, discoveries for the next planner, gate
  status), flips the row to `done`, commits, pushes. Emits
  `<<PHASE_COMPLETE>>`.
- No workable row left → the PLAN step judges the success criteria: met →
  sets the outcome's Status to `done`, emits `<<OUTCOME_COMPLETE>>`; not met
  and nothing workable → sets Status to `blocked` with a note for a human,
  emits `<<OUTCOME_BLOCKED>>`. On either sentinel the wrapper **advances to
  the next backlog outcome**; it exits only when no workable outcome remains
  (exit 0 if none blocked, exit 2 if any need a human).
- Either step may emit `<<PHASE_BLOCKED>>` (record in the Blocked section,
  flip row to `blocked`, commit) — the loop continues to the next row.

Phase plans are deliberately small (≤120 lines): objective, spec delta if the
phase changes feature behaviour (the spec-first rule applies unchanged), TDD
test list, tasks, verification gate, commit message. They are NOT the full
`/smelt:plan` template — the outcome carries the durable context; the plan
carries one phase's worth.

## How to run it

Same rules as the autonomy loop: **from a real terminal or detached tmux —
never a bare backgrounded Bash call inside a Claude session** — and from the
checkout whose branch it should advance.

```bash
WT=/home/andrew/smelt-sql/.claude/worktrees/incremental3   # the target checkout

# Recommended: dedicated tmux session with auto-restart
tmux new-session -d -s outcome \
  "cd $WT && bash .claude/scripts/outcome-loop-forever.sh"

# Single bounded run, foreground
cd "$WT" && bash .claude/scripts/outcome-loop.sh
```

Stop gracefully with `touch $WT/.claude/outcome.stop` (in-flight step finishes
and commits; exit 3). Tunables mirror the autonomy loop: `MAX_ITERATIONS`
(default 25), `MODEL_PLAN` (opus), `MODEL_IMPL` (sonnet), `PERMISSION_MODE`
(bypassPermissions), `CARGO_BUILD_JOBS` (6), `ITER_MEMORY_MAX`/`_HIGH`
(32G/28G), `ITER_COST_WARN` ($15). Logs: `~/.claude/logs/outcome/`.

Exit codes: 0 = backlog complete; 2 = backlog exhausted with blocked
outcomes (needs a human); 3 = graceful stop; 4 = session/usage limit
(forever-wrapper retries); 1 = infra failure / max iterations.

Before launching: `cat .claude/outcome-backlog`, confirm the branch, and check
no `bash .*outcome-loop` process is already running.

## Design notes

- **Synchronous execution only.** Each step is a single-shot `claude --print`
  invocation — `ScheduleWakeup` and `Monitor` are disallowed on the CLI
  (`--disallowedTools`), and the prompts (`.claude/outcome-implement-prompt.txt`,
  `.claude/outcome-plan-prompt.txt`) explicitly forbid `run_in_background` for
  anything the step needs the result of, since there is no later turn to
  deliver a background-task notification into. Backgrounding a gate and
  ending the turn to "wait for it" strands the step with no sentinel — the
  wrapper pauses on a contract violation and any completed-but-uncommitted
  work sits in the working tree until someone finds and commits it (this
  repeated five times in a row on 2026-08-10 before landing; the fix is the
  explicit foreground-only wording in both prompt files).
- The planner step owns the phase list; the implement step owns one phase.
  Scope control happens at every phase boundary instead of once upfront —
  that is the point of the process.
- Opus plans, Sonnet implements: judgement is spent where its leverage is
  (plan lines are ~100× code lines), mechanical execution goes to the cheaper
  model. Override either with env vars for a harder/easier outcome.
- The two prompts live at `.claude/outcome-plan-prompt.txt` and
  `.claude/outcome-implement-prompt.txt`; the sentinel contract is documented
  in both and greped from the final `.result` only (same discipline as the
  autonomy loop).
- Shared infra (memory-bounded per-iteration scopes, usage logging, 429
  handling, auto-stash + origin/main sync) is inherited from the autonomy
  loop's script; the two loops must not run in the same checkout at the same
  time (both sync/commit/push the tree).
