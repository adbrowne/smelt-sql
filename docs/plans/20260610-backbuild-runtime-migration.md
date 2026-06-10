# Plan: Migrate `smelt backbuild` to `smelt_runtime::execute_project` (BUG-070)

**Date**: 2026-06-10
**Spec**: [`docs/specs/cumulative_aggregate.md`](../specs/cumulative_aggregate.md) §CLI, [`docs/specs/cli.md`](../specs/cli.md) §"`smelt run` vs `smelt backbuild`", [`docs/specs/architecture.md`](../specs/architecture.md) §"Run pipeline parity rule (CLI ↔ UI)"
**Spec diff**: none — implements existing normative spec (ledger BUG-070 in `docs/bug-hunt/2026-05-30-findings.md`)
**Tracking PR / branch**: `worktree-test_features`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec sections named in the header — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-test_features`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/` or a hermetic TempDir workspace.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` — **Run Pipeline Parity** is the load-bearing one here: the goal state is that `commands/backbuild.rs` contributes only argument parsing and reporting, like `commands/run.rs`.
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*.

---

## Context

`commands/backbuild.rs` is the last consumer of the legacy CLI executor path (`compute_backbuild_plans`, CLI-side `inject_time_filter`, `executor::*`). It special-cases incremental models only, so a `cumulative_aggregate` model selected for backbuild silently falls through to a full-refresh `CREATE TABLE AS` instead of the per-partition merge loop that `execute_project` dispatches (`smelt-runtime/src/execute.rs` cumulative branch). `cli.md` §"`smelt run` vs `smelt backbuild`" defines backbuild as run + upstream-closure traversal of the selector targets; the runtime selection layer already expresses upstream closure via `+selector` syntax, and `ExecuteRequest` already carries `start`/`end`/`per_partition`/`dry_run`, so the command can become a thin adapter exactly like `commands/run.rs`.

## Scope

### In scope (spec coverage)
- `cumulative_aggregate.md` §CLI: `smelt backbuild --event-time-start … --event-time-end …` dispatches cumulative models to the per-partition merge loop.
- `cli.md` §"`smelt run` vs `smelt backbuild`": upstream-closure traversal preserved (selectors expanded to their `+`-prefixed upstream-closure form before selection).
- `architecture.md` §"Run pipeline parity rule": backbuild consumes `execute_project`; no parallel compile/execute helper remains in the CLI for this command.

### Explicitly deferred
- Threading the function registry into `compute_backbuild_plans` classification (`incremental_models.md` Known Divergence) — the helper is retired by this plan, which moots the divergence; confirm and update that Known Divergence entry rather than implementing it.
- Any change to `smelt run` or UI behaviour.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | pending  |        |      |
| 2     | pending  |        |      |

### Phase 1: Backbuild through `execute_project`

**Goal.** `smelt backbuild` builds its `ExecuteRequest` (selectors rewritten to upstream-closure form, `--event-time-start/--event-time-end` → `start`/`end`) and calls `smelt_runtime::execute_project`, inheriting cumulative dispatch, batch safety, and schema-evolution gates.

**Pre-conditions.** Tree green; `cargo test -p smelt-runtime --test execute_parity` passes.

**TDD tests to write first.**
- `crates/smelt-cli/tests/backbuild_cumulative_e2e.rs::backbuild_dispatches_cumulative_per_partition_merge` — hermetic TempDir workspace (model the fixture on `examples/` cumulative demos + `crates/smelt-cli/tests/meta_hofs_e2e.rs` staging helpers): a `materialization: cumulative_aggregate` model over a seeded event table; pre-populate a partition *outside* the requested window, run `target/debug/smelt backbuild --event-time-start … --event-time-end … --select <model>`, assert (a) exit 0, (b) the out-of-window partition row **survives** (a full-refresh would have dropped it), (c) in-window partitions hold the merged aggregate values. Red on the legacy path (full-refresh clobbers the out-of-window partition).
- `crates/smelt-cli/tests/backbuild_cumulative_e2e.rs::backbuild_traverses_upstream_closure` — two-model chain `staging → cumulative`; backbuild selecting only the downstream model rebuilds the upstream too (assert upstream table content refreshed), per `cli.md` §"`smelt run` vs `smelt backbuild`". Must pass both before and after (guards the selector-rewrite behaviour).
- Existing `crates/smelt-cli/tests/` backbuild coverage (`rg -l backbuild crates/smelt-cli/tests`) stays green — incremental window semantics unchanged.

**Implementation shape.** Rewrite `backbuild(args, scope)` in `commands/backbuild.rs` on the pattern of `commands/run.rs::run`: workspace load → `build_execute_salsa_db` → `ExecuteRequest { select: selectors.map(to_upstream_closure), start, end, per_partition, dry_run, … }` → `execute_project(...)` with `CliReporter` + `CliBackendFactory`. `to_upstream_closure` prefixes each plain selector with `+` unless it already carries upstream syntax (graph-operator selectors pass through; document the rule in a doc-comment). Delete the legacy plan/execute body.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/commands/backbuild.rs` — full rewrite to the adapter shape
- `crates/smelt-cli/tests/backbuild_cumulative_e2e.rs` — new
- `crates/smelt-cli/src/lib.rs` — only if re-exports must be adjusted for compilation

**Docs touched.**
- `docs-site/docs/reference/cli.md` (or wherever `rg backbuild docs-site/docs` hits) — confirm the backbuild entry describes cumulative + incremental uniformly; adjust if it carries an incremental-only caveat.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified (especially the out-of-window-partition-survives assertion)
- [ ] `cli.md` upstream-closure rule satisfied; `cumulative_aggregate.md` §CLI satisfied
- [ ] Run Pipeline Parity honored — no compile/execute logic left in `commands/backbuild.rs`
- [ ] No scope creep into Phase 2 (legacy helper deletion can wait if other callers exist)
- [ ] Spec + docs-site edits are timeless

**Commit.** `feat(backbuild): route smelt backbuild through execute_project (BUG-070)`

### Phase 2: Retire the legacy backbuild executor path

**Goal.** Remove `compute_backbuild_plans` and any CLI-side executor helpers whose only consumer was backbuild; update the `incremental_models.md` Known Divergence that cited `compute_backbuild_plans` as a stale-classification site.

**Pre-conditions.** Phase 1 done and green.

**TDD tests to write first.**
- This phase is subtractive; the red-green is structural: `rg -n "compute_backbuild_plans" crates/` returns no production hits after, and the full suite + `cargo clippy --all-targets` stay green. Add `crates/smelt-cli/tests/backbuild_cumulative_e2e.rs::backbuild_dry_run_reports_plan_without_executing` (asserts `--dry-run` exits 0 and writes no tables) if `--dry-run` coverage does not already exist — the legacy path owned dry-run printing, so deletion needs a behavioural guard.

**Implementation shape.** Delete `compute_backbuild_plans` (and its module if empty) + dead re-exports in `crates/smelt-cli/src/lib.rs`/`backfill.rs`; sweep `executor::*` helpers for backbuild-only consumers. Update `incremental_models.md` §Known Divergences: the `compute_backbuild_plans` clause of the "non-hot classification call sites" entry is retired (describe remaining state behaviourally).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/src/backfill.rs` / `crates/smelt-cli/src/lib.rs` / `crates/smelt-cli/src/executor.rs` — deletions only
- `docs/specs/incremental_models.md` — Known Divergences freshness edit

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences (behavioural wording, no phase vocabulary)

**Review checklist** (material findings only):
- [ ] No production references to the deleted helpers remain
- [ ] `--dry-run` behaviour guarded by a test
- [ ] Known Divergence edit is timeless and accurate
- [ ] No scope creep

**Commit.** `refactor(cli): retire legacy backbuild executor path (BUG-070)`

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- `cargo test -p smelt-cli --test backbuild_cumulative_e2e`
- `cargo test -p smelt-runtime --test execute_parity`
- `cargo test -p smelt-cli --test example_diagnostics`
- `rg -n "compute_backbuild_plans" crates/` → no production hits
- `/smelt:validate cumulative_aggregate` reports no §CLI drift
