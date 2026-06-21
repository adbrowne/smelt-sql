# Handoff: Autonomy-loop OOM kills — root cause + two-part fix

**Date:** 2026-06-21
**Author:** prior Claude session (in worktree `spec_review`); continue this on **`main`**.
**Status:** Root cause identified. Immediate threat neutralized. Both fixes still TODO.

## TL;DR

The autonomy-loop tmux session keeps dying to **systemd-oomd**. The trigger is **not**
the loop's own work and **not** (primarily) concurrent cargo builds. The dominant
cause observed today: a **single `smelt build` process with a pathological memory
blowup** — one process reached **~52 GB RSS in ~2 minutes and was still climbing** on
a 60 GB box. That drives system-wide memory pressure; systemd-oomd then kills an entire
**cgroup (`tmux-spawn-*.scope`)** on PSI pressure — and it can kill the **wrong**
session. The autonomy loop was being reaped as **collateral damage** from a runaway
`smelt build` running in a *different* project/tmux session.

User decision: do **both** fixes — (1) infra hardening so a runaway process can't take
down whole sessions, then (2) investigate the actual `smelt build` memory blowup.

## Evidence gathered (reproduce to confirm)

1. **oomd is the killer, and it targets tmux scopes** — recurring, as recent as today:
   ```bash
   journalctl --user --since "7 days ago" | grep -iE 'oom|killed'
   # → repeated: tmux-spawn-<uuid>.scope: systemd-oomd killed N process(es) ... result 'oom-kill'
   ```
2. **The 52 GB process** — a single `smelt build`:
   ```bash
   ps -eo rss,comm --sort=-rss | head    # smelt at ~52 GB RSS, everything else < 1 GB
   ```
   - PID 287586, cmd `smelt build`, **cwd `/home/andrew/analysis/sherlock`**
   - Climbed to 54 GB RSS in `01:40` elapsed; lived in `tmux-spawn-018d6fc1-...scope`.
   - **Not** the autonomy loop — the loop runs in `/home/andrew/smelt-sql/.claude/worktrees/spec_review`.
   - Killed it (`kill -KILL 287586`); memory recovered **54 GB → 22 GB used**.
3. **oomd policy** (Ubuntu defaults; nothing custom on `app.slice`/`user@.service`):
   ```bash
   systemctl --user show app.slice | grep -iE 'ManagedOOM|MemoryPressure'
   # ManagedOOMMemoryPressure=auto, MemoryPressureThresholdUSec=200ms
   grep -vE '^#|^$' /etc/systemd/oomd.conf   # all defaults
   # DefaultMemoryPressureDurationSec=30s (commented = default)
   ```
   - oomd acts on **PSI pressure** (`/proc/pressure/memory`), not free pages — it fires
     before a hard 64 GB wall, which is exactly the "killed before it hits 64GB" symptom.
   - There is **no swap problem** per se: 8 GB swap file present, lightly used.

## Root cause

- A single `smelt build` on a real project (`sherlock`) consumes pathological memory
  (50 GB+ and climbing — likely unbounded / O(whole graph in RAM)).
- This is the **real bug**. Even with perfect infra, a build that wants 50 GB will fail
  on most machines and is almost certainly holding far more in memory than it needs.
- The infra failure mode on top of it: oomd kills by **cgroup pressure**, so the victim
  may be an *unrelated* tmux session (the autonomy loop), not the offender.

## Fix Part 1 — Infra hardening (do first; fast, robust)

Goal: a runaway process is killed **individually** by the kernel cgroup-OOM-killer,
*before* oomd reaps a whole tmux session — and the autonomy loop is never collateral.

Candidate measures (evaluate, pick, then implement + document):

- **Cap memory per build/scope.** Run heavy commands (`smelt build`, the autonomy loop)
  under a memory-limited transient scope so the kernel kills just that tree:
  ```bash
  systemd-run --user --scope -p MemoryMax=24G -p MemoryHigh=20G \
    --working-directory="$WT" bash "$WT/.claude/scripts/autonomy-loop-forever.sh"
  ```
  `MemoryHigh` throttles (reclaim) before `MemoryMax` hard-kills — gives a softer landing
  than oomd reaping the scope.
- **Make oomd prefer/avoid the right cgroups.** `ManagedOOMPreference=avoid` on the
  autonomy scope (drop-in or `-p`), and/or `omit`, so the loop isn't chosen as victim.
- **Wrap `smelt build` invocations** (in `autonomy-loop.sh` and any harness that runs
  builds) so each build self-limits memory and dies alone if it blows up.
- **Re-examine `CARGO_BUILD_JOBS`** (currently 6 in `autonomy-loop.sh`) — relevant to the
  *secondary* concurrent-link pressure, not the 52 GB single-process case.
- Confirm via `docs/specs/` / CLAUDE.md "Autonomy loop" section whether the launch
  recipe should be updated to the `systemd-run --scope` form (CLAUDE.md already hints at
  a `ManagedOOMMemoryPressure` exemption; reconcile with these findings).

These tie back to existing memory notes:
`project_tmux_oomd_deaths.md`, `project_autonomy_loop_autostash.md`.

## Fix Part 2 — Investigate the `smelt build` memory blowup (the real bug)

**Repro:** `cd /home/andrew/analysis/sherlock && smelt build` → watch RSS climb past
tens of GB (`watch -n1 'ps -o rss -C smelt'`). Cap it first so it can't take the box:
```bash
systemd-run --user --scope -p MemoryMax=16G bash -c 'cd /home/andrew/analysis/sherlock && smelt build'
```

Investigation directions (start at the run pipeline — see CLAUDE.md "Run pipeline parity"):
- `smelt-runtime` `execute_project(...)` — does it hold **all** `CompiledModel`s / Arrow
  schemas / SQL strings in memory at once for the whole graph? How many models in
  `sherlock`, and how large?
- Planner / logical→physical transform — quadratic blowup or retained intermediate plans?
- Salsa cache growth (`smelt-db`) — is the whole graph's derived state retained?
- DuckDB — are result sets / connections being materialized in-process unnecessarily?
- Type inference over wide schemas.

Method: bisect by model count (does memory scale linearly, super-linearly?), and/or
profile with `heaptrack`/massif under a memory cap. Capture a failing repro/test before
fixing (red-green; systematic-debugging Phase 4). This likely warrants a spec/plan if the
fix changes the run pipeline's materialization strategy.

## Immediate state at handoff

- Runaway `smelt build` (PID 287586) **killed**; box healthy (~37 GB available).
- Autonomy loop (PID 282630, worktree `spec_review`) was still running at handoff — note
  it runs the `worktree-spec_review` branch, *not* main. Decide whether to pause it while
  doing this infra work so the two don't interfere.
- No code changes made yet. Nothing committed.
