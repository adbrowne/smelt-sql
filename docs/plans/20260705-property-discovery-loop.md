# Plan: the property-discovery research loop

- **Date**: 2026-07-05
- **Design (spec-equivalent)**: `docs/research/20260705-property-discovery-loop.md`
- **Motivating paper**: `docs/research/20260705-refresh-as-maintenance-plan.md`
- **Docs**: code-only (research infrastructure; no user-facing docs-site change)
- **Type**: a headless `claude -p` **research loop** that empirically maps which
  `(SQL-construct × upstream-property × technique)` cells hold, by running smelt's own emitted
  incremental maintenance over adversarial run schedules and diffing vs full-refresh (Link C).

This plan is unusual: most of the *research work* is performed **by the loop**, iterating over the
catalog (`docs/research/property-discovery/catalog.jsonl`) one cell at a time. The plan's job is to
(1) build the loop scaffolding + the gating Rust harness so the loop has something to run, and
(2) record the phase/acceptance structure the loop follows. The **catalog is the backlog**: its
first rows are infra-build cells (`P0-*`), then the two seed-bug cells (`SC-1/SC-2`), then the
reachable property grid.

## How the loop runs

```bash
# One bounded run (25 iterations), foreground:
cd /home/andrew/smelt-sql/.claude/worktrees/incremental
DUCKDB_LIB_DIR=/usr/local/lib LD_LIBRARY_PATH=/usr/local/lib \
  bash .claude/scripts/property-loop.sh

# Leave it running across sessions (10-min retry when credits/rate-limit exhaust):
tmux new-session -d -s property-loop \
  "cd $PWD && DUCKDB_LIB_DIR=/usr/local/lib LD_LIBRARY_PATH=/usr/local/lib \
     bash .claude/scripts/property-loop-forever.sh"

# Graceful stop (finishes the in-flight cell, commits, then stops):
touch .claude/property-loop.stop
```

Prerequisite: `DUCKDB_LIB_DIR` + `LD_LIBRARY_PATH` exported (unset ⇒ the loop BLOCKS the cell, never
silently skips — a cell verdict without DuckDB is worthless). The fundamentals autonomy loop must be
**paused** for the duration (they share `worktree-incremental`; never run both — design §6).

## Sentinels (the loop emits exactly one per iteration)

- `<<PROBE_COMPLETE>>` — cell resolved (HOLDS/REFUTED/CONDITIONAL), ledger + test committed. Loop.
- `<<PROBE_BLOCKED>>` — design fork or missing infra (e.g. a source shape the driver can't emit).
  Recorded in the cell + ledger; the loop continues to the next pending cell.
- `<<CATALOG_EXHAUSTED>>` — no `pending` cell remains. The loop stops (exit 2) and surfaces a
  summary for a human to seed the next tranche.

## Per-cell routine (what one iteration does)

1. Read `catalog.jsonl`; pick the first `pending` cell (skip `done`/`blocked`).
2. Build/extend the proof for its layer, **reusing** the real analyzer + `execute_project` harness
   (design §2, §3); author the proptest into the crate the cell names. **Red-green**: the test must
   first fail (or, for a candidate-bug cell, first *reproduce the divergence*) and then be committed
   in a state that encodes the verdict.
3. Run just that test: `cargo test -p <crate> --test <t> <name> --quiet 2>&1 | tail -40`.
4. Write the verdict + witness to `ledger.md`; if REFUTED/CONDITIONAL, add a line to
   `unsupported.md`. Set the cell `done`/`blocked`. Optionally append ≤2 adjacent `pending` cells
   (each naming `appended_from`).
5. `cargo fmt --all`; commit + push; emit one sentinel.

## Progress tracking

| Phase | What | Status |
|---|---|---|
| A | Loop scaffolding: `property-loop.sh`, `property-loop-forever.sh` (10-min retry), `property-loop-prompt.txt`, seed `catalog.jsonl` + `catalog.md`, `ledger.md` + `unsupported.md` scaffolds, `property-experimental-gate.sh` CI grep gate | done (2026-07-05) |
| B (`P0-1`) | **In-process real-planner PBT harness**: drive `smelt-runtime::execute_project` over a temp DuckDB with generator-produced rows + a generated run schedule; read back the table. No hand-injected `WHERE`. Lives in `crates/smelt-cli/tests/property_discovery/` (`link_c_harness.rs` + `model_shapes.rs` single model-SQL catalogue); **not** `smelt-db` tests (dev-dep cycle). The gating deliverable (design §3a). | done (2026-07-05) |
| C (`P0-2/3/4`) | Run-schedule generator (append-late + in-place-update between runs; step-`k` source snapshot) + oracle (`EXCEPT ALL`, all-columns diff, per-cell mode, payload-exclusion rule) + generator `MutationProfile` self-check. | done (2026-07-05) |
| D (`P0-5/6`) | Link-A abstract contract-safety proptest scaffold (the 5 adversarial schedule kinds) + Link-B classification-diagnostic scaffold (analyzer facts vs DuckDB ground truth; emits the skeleton-column floor). | done (2026-07-05) |
| E (`SC-1`,`SC-2`) | The two seed candidate-bugs, end-to-end through Link C — the apparatus's first real verdicts (confirm the bug or refute the hypothesis). | in progress — `SC-1` resolved HOLDS (2026-07-06, hypothesis refuted; see ledger + appended `SC-1b`); `SC-2` pending |
| F | The reachable grid: append-only + mutable-snapshot cells (additive/idempotent/holistic aggs, join enrichment, left-join, running-total, `UNION ALL`, composite-key fan-out). Loop-driven. | pending |
| G | Change-feed / CDF generator sub-task; unblocks the retraction / unbounded-lateness cells that currently BLOCK. | pending |

Phase A is built in this session (the scripts + catalog + scaffolds below). Phases B–D are the
deliberate Rust harness build — the loop's first `P0-*` cells (the prompt instructs it to build
infra before property cells), or a focused `/smelt:implement` pass if you prefer to hand-build the
gating harness first. Phases E–G are loop-driven.

## Phase acceptance (TDD anchors)

- **B**: a proptest that compiles a trivial no-`WHERE` incremental model through `execute_project`
  over a temp DuckDB, materializes it, and reads the result back — asserting the emitted SQL went
  through `derive_model_bounds` (assert the derived filter is present in the plan, not injected).
  Red: no such in-process path exists today.
- **C**: a proptest that (i) delivers a source in two windows with a late row *appended between*
  them and asserts the step-`k` full-refresh snapshot differs from a pre-populated one; (ii) an
  `EXCEPT ALL` comparison that a duplicated identical row makes non-empty (proving multiset
  sensitivity vs plain `EXCEPT`); (iii) a self-check that an `append_only`-declared generator never
  emits an out-of-order / mutated row.
- **D**: a Link-A proptest that folds an idempotent monoid over re-delivered/reordered deltas and
  matches the batch aggregate (HOLDS over N), and a `MIN`-over-retractable schedule that diverges
  (REFUTED witness). A Link-B diagnostic that reports `derive_model_bounds`'s reach for a fixed
  model and compares it to a DuckDB clamp-probe.
- **E**: `SC-1` — a correlated-`EXISTS` model over append-only conversions with a late conversion
  appended within 7 days between runs; assert whether smelt's derived bound clamps it away
  (divergence ⇒ REFUTED = confirmed bug). `SC-2` — a clocked source, in-place UPDATE of an
  already-processed partition between runs; assert whether `WindowForward` misses it.

## Reviewer checklist (applied each cell by the loop's own red-green + this gate)

- The Link-C run went through `execute_project` (real bound derivation), **not**
  `run_incremental_sequence`/`execute_model_incremental` (analyzer bypass — design §2.3/N1).
- The oracle is `EXCEPT ALL` over **all columns** minus only declared-non-deterministic payload
  (N2); the full-refresh baseline is the **step-`k` source snapshot** (N3).
- The cell exercised at least one **seeded construct-specific** adversarial schedule for its known
  hazard, not only Link-A's generic kinds (N4).
- **Production changes (authority updated 2026-07-06 — full autonomy, gated by tests; design §8):**
  a production analyzer/planner fix is allowed when it is red→green AND every touched crate's
  `cargo test -p <crate>` is green (recorded in the ledger) AND fmt/clippy clean. Behaviour-defining
  design (new maintenance semantics, wiring a dormant classifier) is BLOCKed for human review, not
  applied. Production changes are untagged real code; the `EXPERIMENTAL(property-discovery)` tag +
  `property-experimental-gate.sh` apply only to disposable test scaffolding.
- The ledger verdict vocabulary never claims "proven" — HOLDS = "no counterexample over N" (F3).

## Commit messages (per phase)

- A: `chore(property-loop): scaffolding — loop + 10min-retry wrapper, prompt, seed catalog, ledger`
- B: `feat(property-loop): in-process execute_project PBT harness (P0-1)`
- C: `feat(property-loop): run-schedule generator + EXCEPT ALL step-k oracle (P0-2/3/4)`
- D: `feat(property-loop): Link-A safety pre-filter + Link-B classification diagnostics (P0-5/6)`
- E: `test(property-loop): SC-1 correlated-EXISTS bound + SC-2 clocked-mutable WindowForward`

## Blocked phases

_(none yet — the loop appends dated entries here when it records a `<<PROBE_BLOCKED>>`.)_
