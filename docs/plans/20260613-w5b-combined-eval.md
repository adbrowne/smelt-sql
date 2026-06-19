# Plan: W5b — Combined SQL↔Python fixed-point evaluation (D-24, ISOLATED)

**Parent (master plan)**: `docs/plans/20260613-spec-impl.md` — the **isolated, risk-flagged** wave of the spec-remediation backlog. Remediates the single highest-risk decision of the 2026-06-13 review: **D-24** (you chose **B** — SQL `generates: models` generators and Python `@model` generators run in **one combined, fully-interleaved fixed-point loop**, each observing the other's emissions across rounds, over the safe one-directional layering). Isolated into its own sub-plan with **extra review checkpoints** and a **strong block discipline** because it is an architectural change, not a localized fix. Runs **after W5** (which lands the Python-model semantics in place). The autonomy loop works this sub-plan phase by phase.

**Date**: 2026-06-13
**Spec**: `docs/specs/python_models.md` §"Iterative evaluation" (the authoritative combined-loop definition — rounds, byte-equal stabilisation, `path`-then-`name` within-round order, non-convergence = circularity), §"Model name derivation" (`find_models` observes SQL-generator emissions); `docs/specs/meta_language.md` §"Multi-model production" rule 4 "Interleaving with Python models" + rule 5 "Determinism" (the W1–W4 pass re-runs each round; intra-round `smelt.models.*` forbid unchanged); `docs/specs/architecture.md` §"Run pipeline parity rule (CLI ↔ UI)" + §"Workspace loading parity rule" (why Python discovery belongs in `smelt-runtime`'s shared pipeline).
**Spec diff**: `e862ebec..HEAD` — the combined-loop **behaviour** already landed in `python_models.md` / `meta_language.md`. The **runtime-home** for Python discovery (this plan's foundational migration) may warrant a one-line architecture.md touch in P5 (pre-authorized, timeless).
**Tracking branch**: `worktree-spec_review`
**Docs**: code-only for the migration + loop. P5 may touch `architecture.md` (run-pipeline parity note) and `docs-site/` if the reviewer flags a user-facing gap.

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then the spec sections above — they are the correctness oracle; do not re-open the settled decisions. **This is the isolated high-risk wave: bias toward `<<PHASE_BLOCKED>>` over guessing.** Run the next `pending` phase in the Progress-tracking table (skip `done`/`blocked`) per the per-phase routine below. Every phase carries an **extra adversarial review pass** (see Per-phase routine step 3a). If that was the last `pending` phase, flip this sub-plan's Status to `done (<today>)` in the master registry and commit together. Emit exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>` (record + continue), `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>`, or `<<ALL_DONE>>`.

## The crux (read before touching code)

Today the two generator families live on **opposite sides of the Salsa boundary** and do **not** interleave:

- **SQL generators** are **Salsa-tracked** in `smelt-db` (`crates/smelt-db/src/queries/project.rs`): `generator_files()` (W1, ~:590), `evaluate_generator()` (W2, ~:1330, with `workspace_shape_includes_generators = false`, :1385), `emitted_models()` (W3, ~:1637), `models_all_with_generators()` (W4, ~:2325). **One pass**, incrementally cached.
- **Python discovery** is **eager, CLI-side** in `smelt-cli/src/python.rs` (`discover_python_models`, :171-365): a 0..5 rounds loop with `models_equal` convergence.
- **Orchestration** (`smelt-cli` `run_setup.rs:20-79`): SQL generators expand first (Salsa), **then** Python discovery runs eager, seeing raw + emitted SQL models. So **Python sees SQL emissions; SQL generators never see Python emissions** — one-directional, not combined.

D-24 (B) requires a **combined** loop where both observe each other across rounds. That is impossible across the current Salsa-vs-eager split. **The unlock (per the owner's directive): move Python discovery into `smelt-runtime`**, the shared compile+execute pipeline both CLI and UI consume (`architecture.md` §"Run pipeline parity rule"). The combined fixed-point loop then lives in `smelt-runtime`, driving the Salsa `emitted_models` query and Python discovery round-over-round against one growing model set. Moving Python to runtime is foundational here **and** independently valuable: it closes the standing CLI↔UI gap where the UI omits Python models entirely (the BUG-077 class).

**Subtlety that must be preserved (not a bug to "fix"):** the **intra-round** `smelt.models.*` forbid inside generator bodies (`GeneratorBodyForbidsModelReflection`) **stays**. Generators still cannot *reflect* over the current round's shape. What the combined loop adds is **inter-round** growth: a generator's **literal `smelt.<path>` references** (and Python `find_models`) resolve against the model set **accumulated from prior rounds** (`meta_language.md` §"Multi-model production" rule 4). Intra-round-no-reflection and inter-round-do-observe are not in tension.

## Goal

Land the combined, fully-interleaved fixed-point loop, in this order of decreasing safety:

1. **Migrate Python discovery `smelt-cli` → `smelt-runtime`** (parity-preserving; CLI and UI both consume it via the shared pipeline). Landable and valuable on its own.
2. **Drive one combined fixed-point loop** in `smelt-runtime`: each round, re-run the SQL-generator Salsa pass and Python discovery against the current model set; collect emissions keyed by canonical address (frontmatter + SQL content); stop when the set is byte-identical to the prior round; error on non-convergence after the bound.
3. **Inter-round bidirectional visibility**: generators' literal `smelt.<path>` refs resolve to prior-round emissions (Python *and* SQL); Python `find_models` observes SQL emissions (already does). Intra-round `smelt.models.*` forbid unchanged.
4. **Determinism**: fixed within-round evaluation order `path` then `name` (the wide-reflection order) across both families; byte-equal stabilisation criterion.

## Design decisions (resolved — do not re-litigate; D-24 = B + owner directive)

- **D-24 = B.** Fully interleaved combined loop; neither family privileged. (Not the safer one-directional A.) Accepted cost: a stricter determinism obligation (fixed `path`-then-`name` within-round order) to keep the bounded loop reproducible.
- **Python discovery moves to `smelt-runtime`** (owner directive, 2026-06-13). The combined loop is a runtime concern (it sits above `smelt-db`'s Salsa queries and is consumed by CLI + UI via `execute_project`). This aligns with the Run-pipeline-parity invariant and closes the UI-omits-Python gap.
- **Round-driving design (the approach to attempt; block if it doesn't hold).** The loop lives in `smelt-runtime`. Each round: (a) ensure the Salsa workspace inputs reflect the current accumulated model set; (b) run `emitted_models` (Salsa) with generators able to resolve literal refs against the accumulated set (inter-round visibility — flip the inter-round half of `workspace_shape_includes_generators`, keep the intra-round `smelt.models.*` forbid); (c) run Python discovery (now in runtime) against the same set; (d) collect new emissions keyed by canonical address with frontmatter + SQL; (e) compare to prior round byte-for-byte; (f) if stable, done; else accumulate and repeat to the 5-round bound; (g) non-stabilisation → non-convergence (circular) error. **If mutating Salsa inputs per round to grow the set proves unworkable (incremental-invalidation storms, or generators structurally can't take prior-round emissions as inputs), BLOCK with the options below — do not hack around the Salsa model.**
- **Intra-round `smelt.models.*` forbid unchanged** (`GeneratorBodyForbidsModelReflection` stays). Inter-round literal-ref resolution is the growth mechanism.
- **Non-convergence = circularity** (consistent with W5's D-23). Carries the W5 semantics into the combined loop.
- **Migration preserves the W5 Python fixes** — W5 fixed Python semantics in `smelt-cli/python.rs`; this wave relocates that (already-correct) code to `smelt-runtime`. Migrate, don't re-fix.

## Per-phase routine
1. **Pre-flight.** `cargo test --quiet 2>&1 | tail -40`. Red on this phase's own target → proceed; unrelated red → block.
2. **Red-green `/smelt:implement`.** Failing test(s) first, then implementation, spec as oracle. Implementer then reviewer.
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings); `cargo test` green; the **run-pipeline parity gate** `cargo test -p smelt-runtime --test execute_parity`; the dual gate `cargo test -p smelt-cli --test example_diagnostics` + `cargo test -p smelt-lsp --test example_workspaces`; scoped `example_builds` for generator + Python fixtures.
3a. **Extra adversarial review (isolated-wave requirement).** After the standard reviewer, run a second reviewer pass explicitly hunting: a non-determinism hole (any order not pinned to `path`-then-`name`), a convergence/termination hole (a set that grows unbounded but isn't caught at the bound), a Salsa-purity or incremental-invalidation regression, and a CLI↔UI divergence (one path runs the combined loop, the other doesn't). Material findings block the phase commit until resolved.
4. **Record + commit.** Row `done` + date; commit + push tests + impl + table with the phase's commit message. Emit `<<PHASE_COMPLETE>>` (or roll-up on the last phase).

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue; bias toward blocking)
This is the isolated high-risk wave: **when in doubt, block with options rather than guess.** Set the row `blocked` + one-line reason; append a dated §"Blocked phases" entry (phase id, reason, candidate options for a human design call); restore a clean committed tree; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:
- The per-round Salsa-input-mutation approach doesn't hold (invalidation storms; generators can't structurally consume prior-round emissions as inputs) — block with the candidate options (e.g. (i) accumulate emissions as an explicit Salsa input the generator pass reads; (ii) run the generator pass non-incrementally inside the runtime loop and accept the LSP-caching cost; (iii) a hybrid where only inter-round literal-ref resolution reads an accumulated-set input).
- The migration (P1) can't preserve CLI↔UI parity without a larger `execute_project` refactor than this phase scopes.
- Any determinism/termination hole the spec's `path`-then-`name` + 5-round bound doesn't actually close for a real case.
- Pre-flight red on unrelated breakage; tree can't return to green.

## Progress tracking

| Phase | Title | Status | Closes | Commit | Date |
|-------|-------|--------|--------|--------|------|
| P1 | Migrate Python discovery `smelt-cli` → `smelt-runtime` (CLI + UI consume via shared pipeline) | done (3c846c6d) | D-24 (1/5) | refactor(runtime): move Python @model discovery into smelt-runtime; CLI + UI consume it (D-24) | 2026-06-19 |
| P2 | Combined fixed-point loop driver in runtime (rounds, growing set, byte-equal convergence, path-then-name order) | done (e8a9cbd1) | D-24 (2/5) | feat(runtime): combined SQL-generator + Python fixed-point evaluation loop (D-24) | 2026-06-20 |
| P3 | Inter-round visibility: generator literal refs resolve to prior-round emissions; intra-round smelt.models.* forbid kept | pending | D-24 (3/5) | feat(db): generators resolve literal refs to prior-round emissions in the combined loop (D-24) | |
| P4 | Bidirectional cross-type tests + non-convergence/circular error | pending | D-24 (4/5) | test(runtime): bidirectional Python↔SQL-generator references; combined-loop non-convergence error (D-24) | |
| P5 | Close-out: CLI↔UI parity gate + architecture note + registry + ROADMAP | pending | D-24 (5/5) | docs(spec-impl): close out W5b — combined evaluation landed; parity gate, registry, roadmap | |

**Status values**: `pending`, `done`, `blocked`. Given the risk, a `blocked` phase here is an expected outcome, not a failure — it surfaces a design call for the owner.

---

### Phase P1: Migrate Python discovery → `smelt-runtime`

**Goal.** Python `@model` discovery lives in `smelt-runtime` and is consumed by both CLI and UI through the shared pipeline — behaviour-preserving, and the UI now runs Python models (closing the omits-Python gap). No combined loop yet.

**Pre-conditions.** W5 done (Python semantics fixed in `smelt-cli/python.rs` — this migrates that code).

**TDD tests to write first:**
- `crates/smelt-runtime/src/.../tests::python_discovery_runs_in_runtime` — the moved `discover_python_models` produces the same `ModelFile` set in runtime as the CLI did (port the `python.rs` unit tests).
- `crates/smelt-runtime/tests/execute_parity.rs::ui_path_runs_python_models` — assert the UI/`execute_project` path now surfaces Python-derived models (it omits them today — the BUG-077 class).
- `crates/smelt-cli/...::cli_python_unchanged` — CLI behaviour byte-identical via the runtime entry.

**Implementation shape.** Move `discover_python_models` and its helpers (`build_project_context`, `run_python_model`, rounds loop) from `crates/smelt-cli/src/python.rs` into `smelt-runtime`; have `smelt-cli`'s `run_setup.rs` and the UI path both call the runtime entry. Respect the run-pipeline-parity rule (`smelt-runtime` internals `pub(crate)`; consumers reach it via `execute_project`). Keep the loop shape identical (one pass SQL-then-Python) — the combine is P2.

**Critical files.** `crates/smelt-runtime/src/...` (new Python module), `crates/smelt-cli/src/python.rs` + `run_setup.rs` (delegate to runtime), the UI path that builds the model set, `python/smelt/` unchanged.

**Review checklist (+ adversarial pass):** Python discovery is in runtime; CLI byte-identical; UI now runs Python models; `execute_parity` green; no `pub` leak of runtime internals; Salsa boundary respected.

**Commit.** `refactor(runtime): move Python @model discovery into smelt-runtime; CLI + UI consume it (D-24)`

---

### Phase P2: Combined fixed-point loop driver

**Goal.** One bounded loop in `smelt-runtime` runs the SQL-generator pass and Python discovery each round against the current accumulated model set, stopping when the set is byte-identical to the prior round; within-round evaluation order is `path` then `name`.

**Pre-conditions.** P1 (Python in runtime).

**TDD tests to write first:**
- `crates/smelt-runtime/...::combined_loop_converges_single_round_when_independent` — a workspace with independent SQL generators + Python models converges in one round, same result as today.
- `...::combined_loop_within_round_order_is_path_then_name` — co-emitting generators contribute in `path`-then-`name` order (observable in a `reduce(union_all)` / emission order).
- `...::combined_loop_byte_equal_stabilisation` — re-evaluating an unchanged workspace is byte-equal (determinism).

**Implementation shape.** Implement the round loop per Design decisions: accumulate the model set, run `emitted_models` (Salsa) + Python each round, compare byte-for-byte, bound at 5. **Attempt the Salsa-input-accumulation approach; block with options if it doesn't hold.** Pin within-round order to `path`-then-`name`.

**Critical files.** `crates/smelt-runtime/src/...` (the loop driver), `crates/smelt-db/src/queries/project.rs` (whatever input the accumulated set is fed through).

**Review checklist (+ adversarial pass):** single combined loop; deterministic `path`-then-`name`; byte-equal stabilisation; no order left unpinned; Salsa incremental behaviour not wrecked.

**Commit.** `feat(runtime): combined SQL-generator + Python fixed-point evaluation loop (D-24)`

---

### Phase P3: Inter-round visibility

**Goal.** Across rounds, a generator's literal `smelt.<path>` references resolve to models emitted by *either* family in a prior round; Python `find_models` observes SQL emissions (already does). The **intra-round** `smelt.models.*` forbid inside generator bodies is unchanged.

**Pre-conditions.** P2 (the loop exists).

**TDD tests to write first:**
- `crates/smelt-db/...::generator_literal_ref_resolves_to_prior_round_python_emission` — a SQL generator with a literal `smelt.<path>` ref to a Python-emitted model resolves in round N+1.
- `...::intra_round_models_reflection_still_forbidden` — `smelt.models.*` inside a generator body still emits `GeneratorBodyForbidsModelReflection` (no regression).
- `crates/smelt-runtime/...::python_find_models_sees_sql_emission` — a Python `find_models` observes an SQL-generator emission (lock the existing direction).

**Implementation shape.** Flip the **inter-round** half of generator visibility (`workspace_shape_includes_generators` / the reference-resolution input) so literal refs resolve against the accumulated set, while leaving the intra-round `smelt.models.*` forbid intact. Keep `evaluate_generator` pure; the accumulated set arrives as an input, not a side channel.

**Critical files.** `crates/smelt-db/src/queries/project.rs` (`evaluate_generator` reference-resolution input), `crates/smelt-runtime/src/...` (feeding the accumulated set in).

**Review checklist (+ adversarial pass):** inter-round literal refs resolve both directions; intra-round reflection still forbidden; `evaluate_generator` stays pure (Salsa-purity rule); no accidental intra-round reflection leak.

**Commit.** `feat(db): generators resolve literal refs to prior-round emissions in the combined loop (D-24)`

---

### Phase P4: Bidirectional tests + non-convergence error

**Goal.** Cover the cross-type cases the codebase has no tests for today, and confirm non-convergence is a clean circular-meta-dependency error.

**Pre-conditions.** P2–P3.

**TDD tests to write first:**
- `crates/smelt-cli/tests/...::python_consumes_sql_generator_emission_e2e` and `...::sql_generator_consumes_python_emission_e2e` — full build of a workspace where each family consumes the other's emission; both succeed and produce correct SQL.
- `...::combined_loop_non_convergence_errors` — a set that oscillates / grows unbounded past 5 rounds → non-convergence (circular) error, anchored sensibly.
- An example workspace `examples/combined_generators_*/` exercising both directions, green under the dual gate.

**Implementation shape.** Mostly tests + fixtures over P2/P3 machinery; wire the non-convergence error to the combined loop's bound (carry W5's D-23 semantics). Fix any gaps the cross-type tests surface.

**Critical files.** `crates/smelt-cli/tests/...`, `examples/combined_generators_*/**`, the combined-loop error path in `smelt-runtime`.

**Review checklist (+ adversarial pass):** both cross-type directions build; non-convergence errors at the bound; example fixtures green; no false non-convergence on a legitimately-growing-then-stable set.

**Commit.** `test(runtime): bidirectional Python↔SQL-generator references; combined-loop non-convergence error (D-24)`

---

### Phase P5: Close-out

**Goal.** Lock CLI↔UI parity on the combined loop, record the runtime-home in the spec if needed, roll up.

**Pre-conditions.** P1–P4 done.

**TDD tests to write first:** none new — runs the parity + dual gates.

**Implementation shape.** `cargo test -p smelt-runtime --test execute_parity` green (CLI and UI both run the combined loop via `execute_project`). If the runtime-home for Python discovery is not yet reflected in `architecture.md` §"Run pipeline parity rule" / §"Workspace loading parity rule", add a timeless sentence (pre-authorized spec touch). Retract any now-satisfied Known-Divergence note (the one-directional-layering note, the UI-omits-Python note). Flip the master registry W5b row to `done (<today>)`; add a `docs/ROADMAP.md` line.

**Critical files.** `docs/specs/architecture.md` (parity note only, if needed), `docs/specs/python_models.md`/`meta_language.md` (KD retraction only), `docs/plans/20260613-spec-impl.md`, `docs/ROADMAP.md`.

**Review checklist (+ adversarial pass):** parity gate green (no CLI↔UI divergence); spec note timeless; retractions genuinely satisfied; registry row `done`; ROADMAP updated.

**Commit.** `docs(spec-impl): close out W5b — combined evaluation landed; parity gate, registry, roadmap`

---

## Deferred during implementation

(Append-only.)

- If P2/P3 block on the Salsa round-driving design, the chosen resolution (explicit accumulated-set input vs non-incremental runtime pass vs hybrid) is a design call for the owner — record options in §"Blocked phases", do not pick one autonomously.

## Blocked phases

Append-only log. None yet. (Expected to be exercised here — this is the isolated high-risk wave.)

## Verification

- `cargo test -p smelt-runtime --test execute_parity`, `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-lsp --test example_workspaces` green; scoped `example_builds` for combined-generator + Python fixtures green.
- Manual smoke: a workspace where a Python `@model` queries a SQL-generator emission *and* an SQL generator references a Python-emitted model builds correctly and deterministically (byte-equal on re-run); the UI run surfaces Python models; an oscillating set errors with a non-convergence diagnostic.
- `/smelt:validate python_models` and `/smelt:validate meta_language` report no behavioural drift on the iterative-evaluation surface.
