# Autonomy loop — operator guide

Moved from `CLAUDE.md` (2026-07-08) to keep the per-turn agent context lean.
This is human-operator documentation for the headless plan-execution loop in
`.claude/scripts/autonomy-loop.sh` / `autonomy-loop-forever.sh`.

The autonomy loop drives the work headlessly. The work is **two-level**:
`.claude/active-plan` names a `master_plan` (the top-level feature backlog —
the feature sweep, whose bug ledger is the master to-do list) and an
`active_subplan` (the focused remediation plan the loop is currently working).
Each iteration spawns a fresh `claude --print`, finds the next `pending` phase
of the active sub-plan (skipping `done`/`blocked` rows), executes it, and emits
a sentinel the wrapper greps to decide what to do next:

- `<<PHASE_COMPLETE>>` — phase committed; loop again.
- `<<PHASE_BLOCKED>>` — **record and continue (no hard-stop)**. On a design
  decision, an unrelated red baseline, or an implementation it can't land green,
  the agent marks the phase row `blocked`, appends a dated entry to the
  sub-plan's "## Blocked phases" section, commits, and the loop moves to the
  next `pending` phase. Blocks are reviewed by a human later, not stop-the-line.
- `<<SUBPLAN_ADVANCED>>` — the sub-plan is exhausted; the loop rolled up to the
  master and advanced `active_subplan` to an existing sibling sub-plan with
  pending work (conservative roll-up — it **never** scaffolds a new sub-plan or
  authors specs autonomously).
- `<<MASTER_EXHAUSTED>>` — sub-plan exhausted and no sibling sub-plan has pending
  work; the loop stops and surfaces a master-level summary for a human to
  scaffold the next sub-plan. (Exit code 2.)
- `<<ALL_DONE>>` — master backlog fully remediated. (Exit code 0.)

Legacy `<<PAUSE_FOR_HUMAN>>` is treated as `<<PHASE_BLOCKED>>` (record +
continue). Only wrapper-level infra failures (merge conflict, dirty tree, claude
crash, missing sentinel) halt the loop. The control loop runs on **Sonnet** by
default (`MODEL=sonnet`; override with `MODEL=`). `autonomy-loop-forever.sh`
wraps `autonomy-loop.sh` and restarts it after `MAX_ITERATIONS` or a crash —
but NOT on exit 0 (`<<ALL_DONE>>`), exit 2 (`<<MASTER_EXHAUSTED>>` — needs a
human to scaffold the next sub-plan; restarting would re-poll the same state
every 10 minutes), or exit 3 (graceful stop). It also halts after
`MAX_FAST_FAILS` (default 3) consecutive failures within `FAST_FAIL_SECS`
(default 120s) — the "claude crashes instantly on every restart" pattern.

Before each iteration the wrapper pre-scans the registry + Progress tables
(`phase_hint` in `autonomy-loop.sh`) and appends a WRAPPER PRE-SCAN HINT to
the prompt naming the ready sub-plan + phase, so the agent verifies one table
instead of reading the ~45KB master plan. The hint is advisory; the tables
win on disagreement. Each iteration's cost is checked against
`ITER_COST_WARN` (default $15) and a warning is printed when exceeded.

## How to run it (correctly)

**Run it from a real terminal or a detached tmux/systemd unit — never from
inside a Claude session, and never by asking Claude to launch it with Bash.**

**Run it from the checkout whose branch you want it to advance — usually a
git worktree, not the main repo root.** The loop is worktree-aware *by
location*: `autonomy-loop.sh` derives `REPO_ROOT` as two levels up from the
script's own path, and every `git merge origin/main` / commit / push acts on
whatever working tree the script lives in. It also reads *that* tree's
`.claude/active-plan`, prompt, and log config. So launch the copy of the
script inside the checkout you want driven. For the current diagnostic-parity
work that is the worktree
`/home/andrew/smelt-sql/.claude/worktrees/test_features` (branch
`worktree-test_features`) — running the main-repo copy at
`/home/andrew/smelt-sql` would resolve `REPO_ROOT` to the main checkout (a
different branch with a different active-plan) and push the wrong branch. The
commands below use a `WT=` variable so you can point it at whichever checkout
is correct.

Why this matters: the loop *is* a chain of `claude --print` subprocesses. If
you ask an interactive Claude session to `setsid nohup bash …` the loop, you
nest a Claude session inside a Claude session, and the loop lives in the
parent session's process group / cgroup. When the harness (or the
auto-retry launcher) restarts or resumes that session, the whole tree is torn
down — the loop receives SIGTERM and dies mid-iteration. (This is exactly how
a launch on 2026-05-31 self-terminated: the wrapper logged `Interrupted by
user` / `Terminated` the moment the parent session was resumed.)

Correct invocations, in order of preference:

```bash
# Point this at the checkout you want the loop to drive (worktree for the
# current diag-parity work; the main repo root only if that's where the
# target branch + active-plan live):
WT=/home/andrew/smelt-sql/.claude/worktrees/test_features

# 1. Dedicated tmux window (recommended — survives your SSH session,
#    matches the cgroup the memory sampler is written to watch):
tmux new-session -d -s autonomy \
  "cd $WT && bash .claude/scripts/autonomy-loop-forever.sh"
tmux attach -t autonomy        # watch it; detach with Ctrl-b d

# 2. Single bounded run (no auto-restart), foreground in a terminal:
cd "$WT" && bash .claude/scripts/autonomy-loop.sh

# 3. Fully detached from any login session via systemd-run:
systemd-run --user --unit=smelt-autonomy --working-directory="$WT" \
  bash "$WT/.claude/scripts/autonomy-loop-forever.sh"
journalctl --user -u smelt-autonomy -f
```

Tunables (env vars): `MAX_ITERATIONS` (default 25), `ITER_COST_WARN`
(default 15 USD), `MAX_FAST_FAILS` / `FAST_FAIL_SECS` (forever-wrapper
crash-loop guard, default 3 / 120s), `PERMISSION_MODE`
(default `bypassPermissions`), `MODEL` (default `sonnet`), `CARGO_BUILD_JOBS`
(default 6 — caps link-time RSS spikes that previously tripped systemd-oomd),
`ITER_MEMORY_MAX` / `ITER_MEMORY_HIGH` (default `32G` / `28G`).

**Per-iteration memory isolation (infra hardening).** Each iteration's `claude`
(and the cargo/smelt builds it spawns) runs inside its own transient
`systemd-run --user --scope` bounded by `ITER_MEMORY_HIGH` (soft, reclaim) and
`ITER_MEMORY_MAX` (hard). A runaway iteration is therefore killed **alone** by
the kernel cgroup OOM-killer before systemd-oomd reaps a whole tmux pane on
memory *pressure* (which it chooses by cgroup and can land on an unrelated
session — the original collateral-kill bug). The scope also sets
`ManagedOOMPreference=avoid` so oomd spares these capped, well-behaved scopes
when some *other* process drives systemwide pressure. The supervisor stays
outside the scope and restarts after a kill. On a host without `systemd-run`,
or an older systemd that rejects a property, the script degrades (caps-only,
then inline-uncapped) rather than failing. This complements the framework-side
fix that bounds DuckDB's own `memory_limit` by default (see
`docs/specs/smelt_yml.md` §Semantics — a single `smelt build` could otherwise
consume ~80% of host RAM and tip the box into pressure;
`docs/handoffs/2026-06-21-autonomy-loop-ooms.md`).

Before launching, check nothing is already running and that the active plan
is the one you want:

```bash
ps -eo pid,etime,args | grep -E 'bash .*autonomy-loop' | grep -v grep   # expect no output
cat "$WT/.claude/active-plan"                                           # confirm in_repo_plan
git -C "$WT" branch --show-current                                      # confirm the branch the loop will push
```

Note: a bare `ps … | grep autonomy-loop` will also match an interactive
Claude session whose launch argument contains the script path (the
auto-retry launcher runs `claude … start .claude/scripts/autonomy-loop-forever.sh`).
Match `bash .*autonomy-loop` specifically so you don't false-positive on the
session and skip a real launch.

Logs land in `~/.claude/logs/diag-parity/iter-*.log` (per-iteration
stdout+`.memory.log`); the forever-wrapper's own output goes wherever you
redirect it. Stop it with `tmux kill-session -t autonomy` (or
`systemctl --user stop smelt-autonomy`, or Ctrl-C in the foreground case).

**Asking Claude to start it:** if you want *me* to kick it off, give me the
prompt below. I will run it in a detached tmux session (option 1) so it
survives this conversation, never with a bare backgrounded Bash call.

> Start the autonomy loop in a **detached tmux session** named `autonomy`
> by running `.claude/scripts/autonomy-loop-forever.sh` from **this
> worktree** (the checkout you're currently in — `cd` into its root, do not
> use the main repo path, since the loop pushes the branch of whatever
> checkout it runs in). First confirm no `bash …autonomy-loop` process is
> already running (a bare grep will match my own session — ignore that), and
> echo the active plan and current branch from this checkout. Do **not**
> launch it with `setsid`/`nohup`/`&` inside this session — it must outlive
> this conversation. After starting, show me `tmux ls` and the first few
> lines of the iteration log to confirm it's iterating.

## Soak sessions

A **soak session** is a deeper, one-off (or periodic) pass of a generative
regression gate at a much larger sample depth than its default per-`cargo
test` run — not a phased implementation plan, so it does not go through the
`pending`/`done` phase state machine above. Register a soak target as a
short stub entry in `.claude/active-plan`'s comment header (next to the
`ACTIVE`/`PAUSED` entries) naming the gate's env-scaled depth knob and its
default; a human (or a `/loop`-style recurring invocation) runs it directly,
outside the phase loop, and files any shrunk failure it finds as a pinned
regression test in the owning plan's test suite — the loop itself never
scaffolds soak sessions on its own.

**Registered soak targets:**

- `cargo test -p smelt-cli --test maintenance_conformance` — the
  maintenance-conformance equivalence gate
  (`docs/specs/maintenance_plan.md` §"The equivalence invariant"). Default
  depth is small (`SMELT_CONFORMANCE_CASES`, unset → 12); a soak pass runs
  `SMELT_CONFORMANCE_CASES=200 cargo test -p smelt-cli --test
  maintenance_conformance --quiet 2>&1 | tail -40`. Automated nightly via the
  `maintenance-conformance-soak` job in `.github/workflows/compat.yml`
  (schedule-gated, or the `run-extended-tests` PR label); this stub is the
  same invocation for an ad hoc local/loop-driven pass.
