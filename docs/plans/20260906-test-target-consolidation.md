# Plan: Consolidate integration test targets

> **RETIRED 2026-09-06 — do not execute. Not superseded by another plan; the
> problem it was written to solve does not exist.**
>
> Every cost figure below was measured while the dev box's SSD was writing at
> ~15 MB/s (TRIM starvation on a drive churning ~300 GB/day). The build was
> stalled on disk writeback — `vmstat` showed 170–450 tasks blocked, 99%
> iowait, ~0% user — so the numbers describe how many bytes each target wrote
> and how contended the disk was, not how much rustc worked.
>
> After `fstrim`, on the same checkout and toolchain:
>
> | Claim in this plan | Re-measured |
> |---|---|
> | 28,539 CPU-seconds for a clean `cargo test --no-run` | **95s wall / 16m21s CPU**, fully cold, `-j32` |
> | 65.6% of it is the 323 integration test targets | one crate's 63 test targets rebuild in **6s** |
> | a 4-line test target costs 70s with deps warm | **~1.3s** total rustc wall |
> | mold is 45% slower | mold is 15% *faster*; linking is ~1.3s either way |
>
> Folding 313 targets into 58 would therefore buy a few seconds, against the
> cost of 46 rewritten command strings across `CLAUDE.md`, four specs and six
> workflows, plus a permanent ratchet gate and a new architecture invariant.
> The premise — "the cost is per-target overhead, not test content" — was
> true only of bytes written.
>
> Kept as a record of the measurement error. The general lesson now lives in
> `CLAUDE.md` §"`cargo --timings` durations are wall time, not build cost".
> If integration-test build time ever becomes a real problem, re-measure from
> scratch rather than reviving this plan's numbers.

**Date**: 2026-09-06
**Spec**: [`docs/specs/architecture.md`](../specs/architecture.md) — adds one invariant under §"Constraints & Invariants"
**Spec diff**: new invariant, authored in Phase 8 (no existing spec text changes)
**Tracking PR / branch**: `worktree-build-speed` (retired, never executed)
**Docs**: code-only — no `docs-site/` surface changes; the one normative addition is the architecture.md invariant in Phase 8

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. The correctness oracle for this plan is **not** a spec section — it is the test inventory built in Phase 1: the set of test names collected by `cargo test -- --list` must be byte-identical before and after every phase. This is a pure move; no test's behaviour may change.
2. Confirm you are on branch `worktree-build-speed`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The test inventory differs before/after a fold and the cause is not an understood rename.
- A folded module cannot compile without editing test *logic* (as opposed to `#[path]`/`include_str!` fixups).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.
- A crate's suite target compiles so slowly that it becomes the critical path (see "Deferred during implementation").

**Conventions every phase:**
- Red-green TDD: the inventory-diff check and the ratchet gate are the failing tests; they go green when the fold is correct.
- Verification gate is `bash .claude/scripts/verify-phase.sh`.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: one crate's fold per phase, no opportunistic test rewrites.
- **Moves only.** A phase may add `mod` declarations, `#[path]` attributes, and fix relative `include_str!` paths. It may not rename a test, change an assertion, delete a test, or "tidy" test code. If a test looks wrong, note it under "Deferred during implementation" and leave it.

---

## Context

A clean `cargo test --no-run` costs 28,539 CPU-seconds, of which **65.6% (18,727s) is compiling the 323 integration test targets** — measured with `cargo --timings` on 2026-09-06. The cost is per-target overhead, not test content: a 4-line `assert_eq!(2 + 2, 4)` test target in `smelt-runtime` takes 70s to build with all dependencies already compiled, against an 86s mean for that crate's real test targets. Cargo compiles every top-level `.rs` file in `tests/` as its own crate, so each one independently monomorphizes and codegens the generic surface of its dependency graph (DuckDB, Arrow, salsa, tokio). Linking is ~2s of that 70s and is not the problem.

Cargo only auto-discovers `.rs` files at the *top* of `tests/`; files in subdirectories are invisible to it. Folding `tests/foo.rs` → `tests/suite/foo.rs` with a `tests/suite.rs` declaring `mod foo;` therefore collapses N targets into 1 while leaving the test code itself untouched.

## Scope

### In scope
- Fold 276 of 313 test targets into one `suite` target per crate: **313 → 58 targets** (37 protected + 21 suites).
- Keep the 37 targets that are referenced by name in `CLAUDE.md`, `docs/specs/`, `.github/workflows/`, `.claude/scripts/` or `scripts/` as standalone targets, so every documented `--test <name>` command keeps working verbatim.
- A ratchet gate that stops the target count regrowing.
- Land the resulting rule as an architecture invariant.

### Explicitly deferred
- **Folding the 37 protected targets.** They cost ~2,100 CPU-seconds combined (~7% of the build). Folding them would mean rewriting ~46 command strings across CLAUDE.md, four spec files and six workflows — a much larger blast radius for a fraction of the win. Revisit only if the measured result in Phase 8 disappoints.
- **Splitting or deleting slow tests.** This plan changes *where code is compiled*, never what it asserts.
- **`cargo-nextest`.** It does not reduce build cost (it still builds every target); its win is on the run side. Separate decision.
- **The mold linker.** Measured slower on this workspace; recorded in CLAUDE.md, not revisited.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | retired  |        |      |
| 2     | retired  |        |      |
| 3     | retired  |        |      |
| 4     | retired  |        |      |
| 5     | retired  |        |      |
| 6     | retired  |        |      |
| 7     | retired  |        |      |
| 8     | retired  |        |      |

---

### Phase 1: Inventory oracle and target-count ratchet

**Goal.** Build the two gates the rest of the plan is verified against: a test-name inventory that must not change, and a per-crate test-target count that may only shrink.

**Pre-conditions.** None.

**TDD tests to write first.**
- `crates/smelt-core/tests/hardening_budget.rs::test_target_ratchet_detects_regression` — writes a temp crate layout with one more top-level `tests/*.rs` than its baseline entry and asserts the gate fails; mirrors the existing `gate_detects_regression` pattern in the same file.
- `crates/smelt-core/tests/hardening_budget.rs::test_target_baseline_is_two_sided` — asserts a count *below* baseline also fails, telling the developer to re-run `--update` (same two-sided rule as `.claude/hardening-baseline.txt`).

**Implementation shape.**
- `scripts/dev/test-inventory.sh` — runs `cargo test --no-run` then each binary with `-- --list`, emitting `<crate>::<target>::<test name>` sorted. Written to a path given as `$1`. This is the move oracle, used by every later phase as `diff before.txt after.txt`.
- `.claude/scripts/test-target-budget.sh` — counts top-level `tests/*.rs` per crate, compares to `.claude/test-target-baseline.txt`, supports `--update`. Modelled on `.claude/scripts/hardening-budget.sh`.
- Seed `.claude/test-target-baseline.txt` with today's counts (the table in "Context").

**Critical files (allowed to touch in this phase).**
- `scripts/dev/test-inventory.sh` — new
- `.claude/scripts/test-target-budget.sh` — new
- `.claude/test-target-baseline.txt` — new
- `.gitignore` — **required**: `.claude/*` is ignored with an explicit whitelist. Add `!.claude/test-target-baseline.txt` or the baseline is silently untracked and CI reads a missing file. Verify with `git ls-files .claude/test-target-baseline.txt` before committing.
- `crates/smelt-core/tests/hardening_budget.rs` — add the two gate tests
- `.github/workflows/test.yml` — run `test-target-budget.sh` in the Lint job, beside `hardening-budget.sh`

**Review checklist** (material findings only):
- [ ] Baseline file is actually tracked (`git ls-files`), not swallowed by `.claude/*`
- [ ] Ratchet is two-sided, matching the hardening-budget convention
- [ ] `test-inventory.sh` output is stable across runs (sorted, no timings, no addresses)
- [ ] No test target folded yet — this phase adds gates only

**Commit.** `test(build): add test-target ratchet and test-name inventory oracle`

---

### Phase 2: Pilot fold — `smelt-dialect`

**Goal.** Establish the fold pattern end-to-end on one small crate (13 targets → 1 protected + 1 suite), and learn the mechanical hazards before scripting.

**Pre-conditions.** Phase 1 gates exist.

**TDD tests to write first.**
- Inventory equality: capture `scripts/dev/test-inventory.sh before.txt` on the parent commit, and assert `diff` against `after.txt` is empty. This is the phase's red-green test.
- `.claude/scripts/test-target-budget.sh` reports `smelt-dialect` at its new lower count and the baseline is updated in the same commit.

**Implementation shape.**
- Move all `crates/smelt-dialect/tests/*.rs` except `emission_ownership.rs` (protected) into `crates/smelt-dialect/tests/suite/`.
- Add `crates/smelt-dialect/tests/suite.rs` with one `mod <name>;` per moved file, sorted.
- Fix module resolution: a moved file's own `mod foo;` now resolves relative to `tests/suite/`, not `tests/`. Where a helper lives outside the suite directory, use `#[path = "../<dir>/mod.rs"] mod <name>;`.
- `tests/snapshots/` stays where it is; check for relative path references from moved files.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-dialect/tests/**` — moves plus `mod` declarations
- `.claude/test-target-baseline.txt` — lower the `smelt-dialect` entry

**Review checklist** (material findings only):
- [ ] `diff before.txt after.txt` is empty — same tests, same names, none lost
- [ ] No test body changed; diff is moves, `mod` lines and `#[path]`/`include_str!` fixups only
- [ ] `emission_ownership` still runs as `cargo test -p smelt-dialect --test emission_ownership`
- [ ] Baseline lowered in the same commit as the fold

**Commit.** `refactor(smelt-dialect): fold 12 test targets into one suite target`

---

### Phase 3: Script the fold, apply to the small crates

**Goal.** Turn the Phase 2 pattern into a repeatable script and apply it to the 13 crates with fewer than 20 targets each (~50 targets folded).

**Pre-conditions.** Phase 2 established the pattern, including the `#[path]` fixups it needed.

**TDD tests to write first.**
- Inventory equality across all crates touched in this phase (as Phase 2).
- `scripts/dev/fold-test-targets.sh --check <crate>` exits non-zero when a fold would change the inventory — a dry-run guard, exercised on one crate in the script's own test.

**Implementation shape.**
- `scripts/dev/fold-test-targets.sh <crate>` — reads the protected list (derived by grepping `--test <name>` across `CLAUDE.md docs/specs .github/workflows .claude/scripts scripts`, never hand-maintained), `git mv`s the rest into `tests/suite/`, generates `tests/suite.rs`, and reports files needing manual `#[path]`/`include_str!` attention rather than guessing.
- Apply to: `smelt-backend`, `smelt-backend-duckdb`, `smelt-backend-spark`, `smelt-backends`, `smelt-bench`, `smelt-core`, `smelt-datagen`, `smelt-fingerprint`, `smelt-oracle-testkit`, `smelt-parser`, `smelt-planner`, `smelt-state`, `smelt-types`, `smelt-ui`.
- `smelt-parser-compat` is skipped entirely — all 6 of its targets are protected.

**Critical files (allowed to touch in this phase).**
- `scripts/dev/fold-test-targets.sh` — new
- `crates/{listed above}/tests/**`
- `.claude/test-target-baseline.txt`

**Review checklist** (material findings only):
- [ ] Protected list is *derived* by grep, not a hardcoded copy that can drift
- [ ] Inventory unchanged for every crate touched
- [ ] Script refuses rather than guesses on relative-path hazards
- [ ] `smelt-parser-compat` untouched

**Commit.** `refactor(tests): fold test targets in the small crates via fold-test-targets.sh`

---

### Phase 4: Fold `smelt-db` and `smelt-lsp`

**Goal.** 37 targets → 6 protected + 2 suites (37 folded).

**Pre-conditions.** Phase 3 script proven.

**TDD tests to write first.**
- Inventory equality for both crates.
- `cargo test -p smelt-lsp --no-default-features --tests` still runs every LSP integration test (this is the CI command; `--tests` must still reach the suite target).

**Implementation shape.**
- Apply the script; expect manual work in `smelt-db`, which has four companion directories (`prop_helpers/`, `proptests/`, `integration/`, `dialect_audit/`) plus `.proptest-regressions` files.
- **Proptest regression files are keyed by target name** (`tests/<target>.proptest-regressions`). Move each alongside its new target name, or the recorded failing cases stop being replayed — a silent loss of coverage. Verify by checking each file is still found after the move.
- Protected in `smelt-db`: `type_property_tests`, `nullability_property_tests`, `dialect_audit`, `proptests`. Protected in `smelt-lsp`: `example_workspaces`, `integration`, `position_encoding`, `property_diff_parity`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/tests/**`, `crates/smelt-lsp/tests/**`
- `.claude/test-target-baseline.txt`

**Review checklist** (material findings only):
- [ ] Every `.proptest-regressions` file still matches a live target name
- [ ] Inventory unchanged for both crates
- [ ] CI's `-p smelt-lsp --no-default-features --tests` command still selects everything it did before

**Commit.** `refactor(smelt-db,smelt-lsp): fold test targets into per-crate suites`

---

### Phase 5: Fold `smelt-logical` (64 targets)

**Goal.** The largest single count: 67 → 3 protected + 1 suite.

**Pre-conditions.** Phase 4 complete.

**TDD tests to write first.**
- Inventory equality for `smelt-logical`.
- `cargo test -p smelt-logical --test walk_coverage` still resolves (protected target, and a standing CI gate as of commit 3582d6ed).

**Implementation shape.**
- Apply the script. Protected: `walk_coverage`, `maintenance_plan_conformance`, `story_coverage`.
- `backbuild_property/` and `backbuild_conformance/` companion directories need `#[path]` fixups from inside the suite.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/tests/**`
- `.claude/test-target-baseline.txt`

**Review checklist** (material findings only):
- [ ] Inventory unchanged
- [ ] The three protected targets still build and run standalone
- [ ] No test body edited

**Commit.** `refactor(smelt-logical): fold 64 test targets into one suite target`

---

### Phase 6: Fold `smelt-runtime` (58 targets)

**Goal.** 63 → 5 protected + 1 suite. This crate has the highest per-target overhead measured (86s mean, 70s of it fixed), so it is the largest single saving.

**Pre-conditions.** Phase 5 complete.

**TDD tests to write first.**
- Inventory equality for `smelt-runtime`.
- All five protected targets still run standalone: `statement_parity`, `execute_parity`, `projection_dialect_invariance`, `dialect_seam`, `restructure_multiplicity` — four of which are standing CI gates.

**Implementation shape.**
- Apply the script. `tests/fixtures/` and `tests/oracle/` are companion directories; `oracle` is declared as `mod oracle;` by several tests and will need `#[path = "../oracle/mod.rs"]` from inside the suite.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/tests/**`
- `.claude/test-target-baseline.txt`

**Review checklist** (material findings only):
- [ ] Inventory unchanged
- [ ] All five protected targets run standalone
- [ ] `cargo test -p smelt-runtime --test statement_parity` timing is not materially worse (it is unchanged code; a regression means the fold pulled extra code into it)

**Commit.** `refactor(smelt-runtime): fold 58 test targets into one suite target`

---

### Phase 7: Fold `smelt-cli` (46 targets)

**Goal.** 59 → 13 protected + 1 suite.

**Pre-conditions.** Phase 6 complete.

**TDD tests to write first.**
- Inventory equality for `smelt-cli`.
- `cargo test -p smelt-cli --no-default-features --features duckdb --test example_diagnostics` still runs (CI command, and `example_diagnostics` holds the one `#[ignore]` in the crate).

**Implementation shape.**
- Apply the script. `smelt-cli` has the most protected targets (13) and five companion directories, including three `maintenance_conformance*` variants that are feature-gated for spark/bigquery — check those still compile under `--features spark,bigquery` per the Lint job's twin-compile check.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/tests/**`
- `.claude/test-target-baseline.txt`

**Review checklist** (material findings only):
- [ ] Inventory unchanged
- [ ] `cargo check -p smelt-maintenance-testkit --features spark,bigquery --all-targets` still passes
- [ ] All 13 protected targets run standalone

**Commit.** `refactor(smelt-cli): fold 46 test targets into one suite target`

---

### Phase 8: Measure, tighten, and land the invariant

**Goal.** Confirm the win, freeze it with the ratchet, and record the rule so new tests default into a suite.

**Pre-conditions.** Phases 2–7 complete.

**TDD tests to write first.**
- `.claude/scripts/test-target-budget.sh` passes at the new counts and fails if a new top-level `tests/*.rs` is added to a folded crate (add one in a temp copy, assert failure).

**Implementation shape.**
- Re-run the Phase 1 measurement on an **idle** machine: `cargo clean && cargo test --no-run --timings`, and record total CPU-seconds, wall time, and the test-target share against the 2026-09-06 baseline (28,539 CPU-sec / 976s wall / 65.6%). Note the load average both times — the baseline was taken under load 90–130.
- Tighten `.claude/test-target-baseline.txt` to final counts.
- Add the invariant to `docs/specs/architecture.md` §"Constraints & Invariants" and the bullet list in `CLAUDE.md`, written timelessly: a new integration test goes in its crate's `tests/suite/` and is declared in `tests/suite.rs`; a *new standalone target* is justified only by being referenced by name from CI or a spec, because each one costs a full re-monomorphization of the crate's dependency graph. Cite the standing gate `bash .claude/scripts/test-target-budget.sh`.

**Critical files (allowed to touch in this phase).**
- `docs/specs/architecture.md` — new invariant (timeless wording)
- `CLAUDE.md` — matching bullet under the invariants list
- `docs/ROADMAP.md` — completion entry with date
- `.claude/test-target-baseline.txt`

**Review checklist** (material findings only):
- [ ] Measured result recorded with the load average, not just a bare number
- [ ] architecture.md wording is timeless — no phase vocabulary, no reference to this plan's structure
- [ ] Ratchet is wired into CI and fails on a new top-level target
- [ ] If the measured saving is materially below ~40%, say so plainly and open the deferred question of folding the 37 protected targets

**Commit.** `docs(architecture): require new integration tests to join their crate's suite target`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the plan is satisfied at the end:
- `scripts/dev/test-inventory.sh /tmp/final.txt` and `diff` against the inventory captured at the parent of Phase 2 — must be empty. No test was lost, renamed or silently skipped.
- `bash .claude/scripts/test-target-budget.sh` — green at the new counts, and fails when a top-level target is added.
- `bash .claude/scripts/verify-phase.sh` — full gate.
- `cargo clean && cargo test --no-run --timings` on an idle box; the test-target share of total CPU-seconds should fall from 65.6% toward ~25%, with total CPU-seconds down roughly 40–50%.
