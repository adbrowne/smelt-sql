# Plan: Model updates — L4 composition for `refresh: cumulative` (rungs 2–4 + local residue)

**Date**: 2026-07-04
**Master plan**: [`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) — the **L4 mode-composition** layer for `refresh: cumulative` of the re-cut master. **Supersedes** the mode-vertical **Group C** (`docs/plans/20260704-model-updates-group-c.md`), which this sub-plan re-cuts onto the fundamentals ladder.
**Specs (oracles)**:
- [`docs/specs/cumulative_aggregate.md`](../specs/cumulative_aggregate.md) — **primary** (the mode's local machinery + composition table). §Composition (the "Properties required" / "Transforms driven" slots this layer wires); §"The maintenance boundary" (where cumulative sits on rungs 1–4 — rung 1 built; rungs 2/3/4 this plan's scope); §"Reprocessing semantics"; §"Interaction with `--auto` / staleness"; §"Cross-partition equivalence" (the end-state specialisation the harness checks); §Surface "Aggregator allowlist" + "Diagnostic codes".
- [`docs/specs/model_maintenance.md`](../specs/model_maintenance.md) — the framework this composition discharges. §"The equivalence invariant" (**ONE** invariant; cumulative is **key-addressed** — identity-requiring `merge_into`, output one row per `unique_key`, the write reaches stored state by key, **equivalence checked on the end-state**); §"The algebraic maintenance ladder" (rungs 1–4 — this plan climbs 2→4); §"Windowed maintenance and the horizon" (windowed-by-default; derived horizon); §"Validator, not chooser" (fail-loud, never downgrade).
- [`docs/specs/model_properties.md`](../specs/model_properties.md) — the derived proofs this layer **consumes by exact name**: **algebraic discriminants** (is-monoid / needs-inverse / decomposable / value-vs-order-monotone), **presentation-map purity**, **driving-fact / anchor resolution**. Built by the fundamentals sub-plan (F4, F7, F2); this layer wires them, never re-derives them.
- [`docs/specs/model_transforms.md`](../specs/model_transforms.md) — the transforms this layer **drives by exact name**: keyed `merge_into` (target-as-replica, built); **windowed-keyed-maintenance driver** (F11); **hidden decomposed state + presentation view** (F12); **retraction via delta history**; **explicit bounded-domain multiset state**. §Constraints "Equivalence or refusal", "The bounded-domain multiset is opt-in and capped".
**Research (the "why" + the L-decomposition)**: [`docs/research/20260704-maintenance-fundamentals.md`](../research/20260704-maintenance-fundamentals.md) — §"Mapping the current master onto the layers" (the row `C1/C2 → L1 rung + L2 presentation-view`; `C3 → L1 inverse-free + L2 delta history`; `C4 → L3 assertion + L2 multiset state`) — the authoritative re-cut this sub-plan realises. Design detail: [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) Part 13 (direct vs hidden state), Part 14 (§14.1 decomposed monoid, §14.2 group/retraction, §14.4 bounded-domain multiset), Part 15 (§15.1 the `(state table + view)` trick portably; §15.3 the two hazards — presentation-view purity + atomic state/view swap).
**Spec diff**: none new — L0 authored `cumulative_aggregate.md` / `model_maintenance.md` / `model_properties.md` / `model_transforms.md`; the rung-2/3/4 placement is already normative in `cumulative_aggregate.md` §"The maintenance boundary" and the maintenance ladder. Each phase **removes/narrows** a §Known-Divergence note and, where pre-authorised, adds an aggregator to `cumulative_aggregate.md` §Surface as its behaviour ships; no phase authors a spec. (The single pre-authorised **new** surface — the bounded-domain-budget declaration CU4 gates on — is **owned by the L3 declarations sub-plan** `docs/plans/20260704-model-updates-l3-declarations.md`, not authored here.)
**Tracking branch**: `worktree-incremental`
**Docs**: code+docs

**Scope boundary (read first).** This sub-plan is the **L4 composition for `refresh: cumulative`**: it wires the fundamentals **L1 proofs** (F4 algebraic discriminants, F7 presentation-map purity, F2 driving-fact resolution) and **L2 transforms** (F11 windowed-keyed-maintenance driver, F12 hidden decomposed state + presentation view) up the algebraic ladder for cumulative, and builds the **cumulative-local residue** the fundamentals layer explicitly left mode-local: the retraction / delta-history state machinery, the bounded-domain multiset state machinery, reprocessing semantics, and presentation-purity **enforcement** on the cumulative view. It **supersedes** the mode-vertical **Group C** (`docs/plans/20260704-model-updates-group-c.md`): Group C's C1–C4 are re-expressed here as CU1–CU4, but the rung *mechanisms* (decomposed-state+view, the discriminant classifier, the driver) that Group C would have built privately are now **consumed from the fundamentals sub-plan by name** — this layer never re-derives a proof or re-builds a transform the fundamentals layer owns. It does **not** cover the fundamentals capabilities themselves (F-phases), the L3 declaration surfaces (the bounded-domain budget — that surface is the L3 sub-plan's), or the sibling keyed modes `latest_value` / `versioned` / `accumulating_snapshot` / `materialized_view` (their own L4 sub-plans, which reuse F11 + F12 the same way).

---

## Execution prompt (for a fresh Claude session / the autonomy loop)

You are executing this plan phase by phase. It is a sub-plan registered in
[`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) §"Spawned sub-plans" (it replaces the
Group C registry row when this L4 layer is scaffolded in — the loop never scaffolds it autonomously).

**Before touching any code:**
1. Read this entire plan, then read the cited spec sections — they are the correctness oracle. The
   invariant for every phase is the **processed-input equivalence invariant** in its **end-state**
   specialisation (`model_maintenance.md` §"The equivalence invariant"): cumulative is **key-addressed**,
   so `cumulative_run(π(S)) == full_refresh(source.where(partition ∈ S))` for any processed partition set
   `S` and any ordering — the maintained-relation equivalence contract **holds unconditionally on every
   rung**; what changes across rungs is the *state representation and its size*, never the fidelity of the
   user value. Do not weaken end-state equivalence to make a rung land.
2. Confirm you are on branch `worktree-incremental`. Confirm this phase's **Depends on** fundamentals
   phases (F-numbers), the L3 bounded-domain-budget phase (CU4 only), and Group A (A1 — `RefreshStrategy::
   Cumulative`) are landed. A capability this layer consumes by name (discriminants, purity, driver,
   decomposed-state+view) must already be `built` in `model_properties.md` / `model_transforms.md`; if it
   is not, block (do not re-derive it here — that is the fundamentals layer's job).
3. Find the next `pending` row in the Progress-tracking table below. That is your phase. Honour its
   **Depends on** field. If every row is `done`, run §Verification, flip this sub-plan's registry Status
   to `done` in the master, and stop.

**Per phase, run `/smelt:implement`'s loop:** pre-flight (`cargo build`/`cargo test` green except this
phase's own red target) → implementer subagent (red-green TDD on the listed tests; **every** phase names a
**fail-closed reject test** and an **end-state-equivalence harness test** extended to the rung's new
aggregators) → reviewer subagent (material findings only) → iterate → set the row `done` → commit + push
with the phase's `Commit.` line. A phase's row lists a **spec increment** where one is pre-authorised;
making the cited `cumulative_aggregate.md` §Surface edits is expected, not scope creep.

**Every phase must not regress the cumulative end-state equivalence harness.** This layer re-expresses
cumulative on the shared driver (F11) and adds state representations behind it. The acceptance gate for
**every** phase includes the existing cumulative integration/equivalence tests
(`crates/smelt-cli/tests/e2e/per_partition_equivalence.rs`,
`crates/smelt-cli/tests/cli_unit/cumulative_equivalence.rs`, `crates/smelt-cli/tests/cumulative*`) staying
green. A phase that flips one of those is a bug in the wiring, not a spec change — do not update the
equivalence expectations to match new output.

**Equivalence-harness tests need DuckDB.** Every phase changes emitted SQL (state columns, presentation
view, delta side table, multiset merge) and asserts end-state equivalence via the DuckDB harness; that
requires `DUCKDB_LIB_DIR` set (and `LD_LIBRARY_PATH`) per `CLAUDE.md`. The pure classifier assertions
(which aggregator classifies to which decomposition / rung) are `smelt-logical` unit tests with no DuckDB
dependency.

**Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this file only*. Spec + `docs-site/`
edits describe the feature as if it always existed; as each rung lands, **remove** the matching
§Known-Divergence note (or narrow it to the rungs still outstanding) and flip the aggregator into §Surface,
rather than annotating either with a phase number.

**Block rule.** On a design decision not answered here or by the spec (see §"Open decisions surfaced for
the implementer"), a dependency capability that is not yet `built`, or a pre-flight red unrelated to this
phase's target: set the row `blocked` with a one-line reason, append to §"Blocked phases", restore a clean
tree, commit, emit `<<PHASE_BLOCKED>>`. Otherwise emit `<<PHASE_COMPLETE>>`.

---

## Context

Cumulative today implements exactly **rung 1** of the algebraic ladder (`model_maintenance.md` §"The
algebraic maintenance ladder"): the direct-monoid allowlist (`SUM`/`COUNT`/`MIN`/`MAX`/`BOOL_*`/`BIT_*`),
where the stored column *is* the answer. The classifier lives in
`crates/smelt-logical/src/rules/cumulative.rs` (`classify_cumulative` `:227`, `combiner_for` `:82`,
`CrossPartitionCombiner` `:43`, `AggregatorColumn` `:30`, `CumulativeClassification` `:183`); the
per-partition merge loop lives in `crates/smelt-runtime/src/cumulative.rs` (`execute_cumulative_aggregate`
`:34`, partition loop `:117`, first-run `create_table_as` `:155`, `build_cumulative_merge_sql` `:218`,
dispatched from `crates/smelt-runtime/src/execute.rs:779`). There is **no** presentation-view machinery on
the cumulative path — the target table *is* the user relation. (A generic `create_view_as` exists —
`crates/smelt-backend/src/lib.rs:37`, DuckDB impl `crates/smelt-backend-duckdb/src/lib.rs:320` — but
nothing on the cumulative path uses it.)

The 2026-07-04 fundamentals-first re-cut (`docs/research/20260704-maintenance-fundamentals.md`) moved the
*shared spine* of rungs 2–4 into the fundamentals sub-plan: **F4** derives the algebraic discriminants
(is-monoid / needs-inverse / decomposable / value-vs-order-monotone) that decide which rung a combiner
sits on; **F7** proves presentation-map purity; **F11** lifts cumulative's per-partition loop into the
mode-agnostic windowed-keyed-maintenance driver; **F12** builds the hidden-decomposed-state + presentation
view mechanism. This L4 sub-plan **wires those by name** for cumulative and builds only what is genuinely
cumulative-local: the delta-history side-table + subtract-then-add reprocessing (rung 3), the
bounded-domain multiset state (rung 4, gated on the L3 budget declaration), reprocessing semantics, and the
enforcement that cumulative's presentation view is `π`-pure. The end-state equivalence contract
(`cumulative_aggregate.md` §"Cross-partition equivalence") is the net across all four rungs; this layer
never trades correctness for a rung, only state size.

## Scope

### In scope (rungs 2–4 + cumulative-local residue, re-cut from Group C)

- **CU1 (was C1) — rung 2 wiring: `AVG` via `(sum,count)` state table + presentation view.** Admit `AVG`
  by driving the **hidden decomposed state + presentation view** transform (F12) — the state table stores
  `(sum, count)` under componentwise `+`, the **presentation view** exposes `sum/count` — with the
  `(state table, view)` treated as one atomically-swapped unit, and the view's `π`-purity **enforced** by
  the presentation-map-purity proof (F7). The **enabling wiring** for the whole rung-2 unlock (CU2 reuses
  it). Cumulative-local residue landed here: presentation-purity **enforcement** on the cumulative view; the
  atomic state-table/view swap on the cumulative execution path.
- **CU2 (was C2) — rung 2 extension: variance/stddev (Welford) + approximate-distinct sketch.** Add the
  Welford-triple state for `VAR`/`STDDEV` and an HLL/sketch register-vector state for approximate
  `COUNT(DISTINCT)`, each with its presentation map, reusing CU1's F12-backed mechanism — new entries in the
  closed decomposition table, no new execution path.
- **CU3 (was C3) — rung 3: group retraction via per-partition delta history + `--auto` fidelity + the
  reprocessing-semantics residue.** For the **group** (invertible) subset `SUM`/`COUNT`/`BIT_XOR` (per F4's
  needs-inverse discriminant), drive the **retraction via delta history** transform — store per-partition
  deltas in a side table, reprocess a changed partition by subtract-then-add — and upgrade `--auto`
  staleness to "exactly the changed partitions" for fully-reversible models. Reprocessing semantics (the
  refuse-vs-retract decision) is the cumulative-local residue landed here. `MIN`/`MAX`/`BOOL_*`/`BIT_AND`/
  `BIT_OR` are monoids-not-groups and still refuse reprocessing (full refresh).
- **CU4 (was C4) — rung 4: opt-in bounded-domain multiset for exact holistic aggregates.** Admit exact
  `MEDIAN`/`PERCENTILE`/`MODE`/quantiles/exact-`COUNT(DISTINCT)`/`DISTINCT`-aggregates by driving the
  **explicit bounded-domain multiset state** transform — store the per-key `value → count` multiset —
  **gated on the bounded-domain-budget declaration owned by the L3 declarations sub-plan**
  (`docs/plans/20260704-model-updates-l3-declarations.md`), with a runtime cap + full-refresh fallback and
  a **default-refuse** fail-loud message when the budget is absent.

### Explicitly deferred

- **General-operator retraction over joins, and `DISTINCT`/exact-distinct whose state is unbounded in a
  dimension the user cannot cap** — not smelt-driven-maintainable; delegated to native IVM via
  `refresh: materialized_view` (its own L4 sub-plan / `materialized_view.md`). CU4 moves the boundary only
  for *single-column* holistic aggregates the user can bound (research §14.3–§14.4).
- **The sibling keyed modes** `latest_value` / `versioned` / `accumulating_snapshot` — separate L4
  sub-plans. They reuse F11 (driver) and F12 (presentation view) exactly as this layer does, but are
  distinct classifiers, not cumulative rungs.
- **A decomposition *registry*.** The decomposed-monoid set stays a **closed, hard-coded rewrite table**
  beside `combiner_for` (`cumulative.rs:82`), matching cumulative's "fixed allowlist, not a registry" stance
  (`cumulative_aggregate.md` §Design). Revisit only on a concrete custom-sketch motivator (research §18.2).

## Progress tracking

| Phase | Depends on | Spec anchor | Status |
|-------|-----------|-------------|--------|
| CU1 | Group A (A1); F4 (decomposable discriminant); F7 (presentation-map purity); F11 (windowed-keyed-maintenance driver); F12 (hidden decomposed state + presentation view) | `cumulative_aggregate.md` §"The maintenance boundary" (rung 2); `model_maintenance.md` §"The algebraic maintenance ladder" rung 2; `model_transforms.md` "Hidden decomposed state + presentation view" | pending |
| CU2 | CU1; F4 (decomposable discriminant) | `cumulative_aggregate.md` §"The maintenance boundary" (rung 2) | pending |
| CU3 | Group A (A1); F4 (needs-inverse / group discriminant); F11 (driver) | `cumulative_aggregate.md` §"Reprocessing semantics", §"Interaction with `--auto` / staleness", §"The maintenance boundary" (rung 3); `model_transforms.md` "Retraction via delta history" | pending |
| CU4 | CU1; F4 (holistic discriminant); L3 bounded-domain-budget declaration (`docs/plans/20260704-model-updates-l3-declarations.md`) | `cumulative_aggregate.md` §"The maintenance boundary" (rung 4); `model_maintenance.md` ladder rung 4; `model_transforms.md` "Explicit bounded-domain multiset state" | pending |

---

### Phase CU1: Rung 2 wiring — `AVG` via `(sum,count)` state table + presentation view

**Goal.** Admit `AVG` by driving the fundamentals **hidden decomposed state + presentation view** transform
(F12) on the cumulative path: store `(sum, count)` in a state table under componentwise `+`, expose
`sum/count` through a presentation view, treat `(state table, view)` as one atomically-swapped unit, and
**enforce** with the presentation-map-purity proof (F7) that the view's `π` is a pure function of a single
consistent state row. This is the **enabling wiring** for the whole rung-2 unlock (CU2 reuses it verbatim).
The rung mechanism (state+view emit) is F12's; this phase is the cumulative wiring + the cumulative-local
residue (purity **enforcement** on the cumulative view, atomic swap on the cumulative execution path).

**Spec anchor.** `cumulative_aggregate.md` §"The maintenance boundary" (rung 2 — the deferred `AVG` rewrite
grows into a decomposed monoid behind a presentation view); §Composition "Transforms driven" (rung-2 slot);
`model_maintenance.md` §"The algebraic maintenance ladder" rung 2; `model_transforms.md` §Semantics "Hidden
decomposed state + presentation view" (sound iff `π` is pure over one consistent state row). Consumed by
name (not built here): F4 decomposable discriminant, F7 presentation-map purity, F11 driver, F12 state+view.

> **Scope note.** CU1 ships **only `AVG`** as the first decomposed aggregator. Variance/stddev and
> approximate-distinct are CU2 — additional entries in the same closed decomposition table, added once the
> wiring this phase lands is proven on `AVG`. Do not add them here.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/cumulative.rs` unit — `AVG(amount)` in a non-key projection now
  **classifies** (today it refuses `CumulativeUnknownAggregator`) via F4's decomposable discriminant: the
  resulting `AggregatorColumn` (`:30`) is a *decomposed* variant carrying state columns `(_sum_amount,
  _count_amount)`, their componentwise `Sum` combiners, and a presentation expression `_sum_amount /
  _count_amount AS avg_amount`. A composite `AVG(x) + 1` still refuses (`CumulativeUnknownAggregator`, not a
  direct call).
- `crates/smelt-logical/src/rules/cumulative.rs` unit — the decomposition table is the single source of
  truth (parallel to `combiner_for` at `:82`); a still-unknown aggregator (`STRING_AGG`) refuses.
- `crates/smelt-runtime/src/cumulative.rs` unit — for an `AVG` model the driver (F11) maintains **both**
  `_sum` and `_count` state columns componentwise, and a presentation-view builder emits `CREATE OR REPLACE
  VIEW <model> AS SELECT <keys>, _sum/_count AS avg_… FROM <model>__state`.
- `crates/smelt-runtime/src/cumulative.rs` unit / integration — the state table and the view are
  created/replaced as **one atomically-swapped unit** (research §15.3 hazard 2): the first-run path
  (`create_table_as` at `:155`) creates the state table then the view in the same step; the view is never
  left pointing at a stale/absent state table.
- **End-state equivalence (DuckDB harness) — must not regress + extended.** Real-fixture e2e: a cumulative
  model with an `AVG` column maintained across ≥3 source partitions has **end-state equality with a full
  refresh** over the union of those partitions; the **existing** rung-1 cumulative equivalence cases stay
  green after re-expressing the path on the F11 driver. Extend
  `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs` (or the sibling
  `crates/smelt-cli/tests/cli_unit/cumulative_equivalence.rs`); add the fixture under `examples/`. Requires
  `DUCKDB_LIB_DIR`.
- **Fail-closed reject test (`π`-purity enforcement).** The classifier admits a decomposition **only** when
  F7 proves its `π` is pure over a single state row: a decomposition whose presentation would reference
  another row / table / window is **refused** (assert the admitted set is exactly the closed table, no
  cross-row `π`). This is the cumulative-local presentation-purity enforcement residue.

**Implementation shape.** Widen `AggregatorColumn` (`cumulative.rs:30`) to be either **direct** (today's
`CrossPartitionCombiner` over the user column, `:43`) or **decomposed** (a set of hidden state columns, each
with its per-partition aggregator + componentwise combiner, plus a presentation SQL expression). Add a
**closed decomposition table** beside `combiner_for` (`:82`) mapping `AVG(x)` → `{state: [SUM(x) AS _sum_x
(Sum), COUNT(x) AS _count_x (Sum)], π: "_sum_x / _count_x"}`, keyed on F4's decomposable discriminant.
`CumulativeClassification` (`:183`) carries the state-table projection + the presentation-view definition,
and calls F7 to gate `π`-purity. Execution: the driver (F11) maintains the **state table** (`<model>__
state`, hidden columns); after the merge loop, (re)create the presentation **view** named `<model>` as the
user-facing relation, atomically with the state table, reusing the already-present `create_view_as`
primitive. Downstream refs resolve to the **view** — `cumulative_aggregate.md` §"Output shape" (keyed
lookup, no partition column) is unchanged; the state table is never a dependency target.

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — `AggregatorColumn` (`:30`), `CrossPartitionCombiner`
  (`:43`), `combiner_for` (`:82`), `CumulativeDiagnostic` (`:100`), `CumulativeClassification` (`:183`),
  `classify_cumulative` (`:227`); F4 discriminant + F7 purity consumed as licences.
- `crates/smelt-runtime/src/cumulative.rs` — `execute_cumulative_aggregate` (`:34`), partition loop
  (`:117`), first-run `create_table_as` (`:155`), `build_cumulative_merge_sql` (`:218`); re-expressed on the
  F11 driver + F12 state/view emit; dispatched from `crates/smelt-runtime/src/execute.rs:779`.
- `crates/smelt-backend/src/lib.rs:37` (`create_view_as`) / `crates/smelt-backend-duckdb/src/lib.rs:320`
  (DuckDB impl) — reuse for the presentation view; no new backend method.
- `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs`,
  `crates/smelt-cli/tests/cli_unit/cumulative_equivalence.rs` — extend for `AVG`.
- `examples/` — the real `AVG` fixture.

**Docs touched.**
- `cumulative_aggregate.md` — **spec increment (pre-authorised)**: add `AVG` to §Surface "Aggregator
  allowlist" as a *decomposed* aggregator (state `(sum,count)`, presented `sum/count`). **Remove** the
  §Known-Divergence "`AVG` rewrite (rung 2)" note and **narrow** "Only the direct-monoid rung is
  implemented" to rungs 3–4 outstanding.
- `model_transforms.md` §Known Divergences — remove "decomposed-state-plus-view" from the "Unbuilt" list
  (F12 flips it; verify it is `built`, not annotated with a phase).
- `docs-site/docs/guide/materializations.md` — note `AVG` is available in cumulative models (timeless prose;
  no "now supports").

**Review checklist.**
- [ ] `AVG` classifies to a decomposed state via F4; `AVG(x)+1` and `STRING_AGG` still refuse.
- [ ] State table + presentation view move as one atomically-swapped unit; downstream sees only the view.
- [ ] The view is emitted through F12 + `create_view_as`; F7 gates `π`-purity (cross-row `π` refused).
- [ ] End-state equivalence extended to `AVG` and green; **existing rung-1 cumulative equivalence unchanged.**
- [ ] Decomposition table is one closed source of truth (no second edit site).
- [ ] Spec/docs edits timeless; `AVG` Known-Divergence removed, rung note narrowed.

**Commit.** `feat(cumulative): rung-2 AVG via decomposed (sum,count) state + presentation view (wires F12/F7)`

---

### Phase CU2: Rung 2 extension — variance/stddev (Welford) + approximate distinct

**Goal.** Add the Welford-triple state for `VAR`/`STDDEV` and an HLL/sketch register-vector state for
approximate `COUNT(DISTINCT)`, each with its presentation map, **reusing CU1's F12-backed `(state table,
view)` wiring** — new entries in the closed decomposition table, no new execution path. Per
`cumulative_aggregate.md` §"The maintenance boundary" (rung 2) and research §14.1.

**Spec anchor.** `cumulative_aggregate.md` §"The maintenance boundary" (rung 2); `model_maintenance.md`
ladder rung 2 (decomposed monoid: `AVG=(sum,count)`, variance/Welford, approx-distinct/HLL). Consumed by
name: F4 decomposable discriminant.

**Pre-conditions.** CU1 landed (the decomposed-`AggregatorColumn` variant, the closed decomposition table,
the F7-gated `π`-purity check, and the F12/F11 execution path all exist and are proven on `AVG`).

**Depends on.** CU1, F4.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/cumulative.rs` unit — `VAR_POP(x)` / `STDDEV(x)` classify (via F4's
  decomposable discriminant) to a `(count, sum, sum_of_squares)` (or numerically-stable Welford) triple
  state with a closed-form `π`; `APPROX_COUNT_DISTINCT(x)` classifies to a sketch register-vector state with
  `π` = cardinality estimate.
- `crates/smelt-runtime/src/cumulative.rs` unit — the variance triple merges componentwise; the sketch
  state merges register-wise (`max` / HLL merge) through the same driver path CU1 built.
- **End-state equivalence (DuckDB harness) — must not regress + extended.** Real-fixture e2e: a
  `VAR`/`STDDEV` cumulative model has **exact** end-state equality with a full refresh (variance is exact
  under componentwise merge); an `APPROX_COUNT_DISTINCT` model is within a documented relative-error
  tolerance of the exact full-refresh distinct count; CU1's `AVG` case and all rung-1 cases stay green.
  Extend `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs`. Requires `DUCKDB_LIB_DIR`.
- **Fail-closed reject test.** **Exact** `COUNT(DISTINCT)` still refuses at CU2 (it is the CU4 opt-in
  multiset, not a bounded decomposed monoid); `STRING_AGG` still refuses. The message points at the
  approximate form or `refresh: full`.

**Implementation shape.** Extend CU1's closed decomposition table with `VAR_*`/`STDDEV_*` → Welford triple
and `APPROX_COUNT_DISTINCT` → sketch register vector; no new execution path — the driver, state table, and
presentation view are CU1's. Settle the **closed-table vs registry** decision (research §18.2) in favour of
the **closed table**. Keep the sketch representation choice inside the decomposition table so the classifier
stays derive-from-SQL (see §"Open decisions").

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — the decomposition table + `AggregatorColumn` variants.
- `crates/smelt-runtime/src/cumulative.rs` — componentwise / register-wise merge through the driver path.
- `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs` — variance + approx-distinct coverage.
- `examples/` — a fixture exercising `VAR`/`STDDEV`/`APPROX_COUNT_DISTINCT`.

**Docs touched.**
- `cumulative_aggregate.md` — **spec increment (pre-authorised)**: add `VAR`/`STDDEV`/`APPROX_COUNT_DISTINCT`
  to §Surface "Aggregator allowlist" as decomposed aggregators; further narrow "Only the direct-monoid rung
  is implemented" toward rungs 3–4.
- `docs-site/docs/guide/materializations.md` — list the new decomposed aggregators (timeless).

**Review checklist.**
- [ ] Variance exact vs full refresh; approx-distinct within the documented tolerance.
- [ ] Exact `COUNT(DISTINCT)` still refuses (deferred to CU4); `STRING_AGG` still refuses.
- [ ] No new execution path — CU1's F12/F11 wiring reused unchanged; **rung-1 + `AVG` equivalence green.**
- [ ] Decomposition stays a closed table (no registry); the sketch choice is documented.
- [ ] Spec/docs edits timeless.

**Commit.** `feat(cumulative): rung-2 variance/stddev Welford + approximate-distinct sketch decompositions`

---

### Phase CU3: Rung 3 — group retraction via delta history + `--auto` fidelity + reprocessing residue

**Goal.** For the **group** (invertible) aggregators `SUM`/`COUNT`/`BIT_XOR` (per F4's needs-inverse
discriminant), drive the **retraction via delta history** transform: store per-partition deltas in a side
table so a changed source partition is reprocessed by **subtract-then-add**; derive reversibility from the
projection; upgrade `--auto` staleness to "exactly the changed partitions" for fully-reversible models.
`MIN`/`MAX`/`BOOL_*`/`BIT_AND`/`BIT_OR` are monoids-not-groups and still require a full refresh to reprocess.
The reprocessing-semantics **decision** (refuse vs retract) is the cumulative-local residue landed here. Per
`cumulative_aggregate.md` §"Reprocessing semantics", §"Interaction with `--auto` / staleness", §"The
maintenance boundary" (rung 3), and research §14.2.

**Spec anchor.** `cumulative_aggregate.md` §"Reprocessing semantics" (refuse-in-v1 → retract-for-reversible);
§"Interaction with `--auto` / staleness" (the reversible-vs-irreversible split); §"The maintenance boundary"
(rung 3); `model_maintenance.md` ladder rung 3 (group / invertible); `model_transforms.md` "Retraction via
delta history". Consumed by name: F4 needs-inverse / group discriminant, F11 driver.

**Pre-conditions.** Group A (A1) landed; F4 needs-inverse discriminant landed; F11 driver landed. CU3
operates on **direct-monoid** columns that are also groups, so it needs **no** presentation view — it does
**not** depend on CU1. Today reprocessing is refused at planning time (`cumulative_aggregate.md`
§"Reprocessing semantics") and `--auto` is conservative.

**Depends on.** Group A (A1), F4, F11.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/cumulative.rs` unit — a model whose every non-key combiner is in the group
  subset (`SUM`/`COUNT` → `Sum`, `BIT_XOR` → `BitXor`) classifies **reversible** via F4's needs-inverse
  discriminant; a model containing `Min`/`Max`/`BoolAnd`/`BoolOr`/`BitAnd`/`BitOr` classifies
  **non-reversible**. Reversibility is *derived from the projection*, never declared.
- `crates/smelt-runtime/src/cumulative.rs` unit — after merging partition `D`, its delta is retained in the
  side table `<model>__deltas`; the reprocessing builder emits subtract (stored delta) then add (recomputed
  delta) using each group combiner's **inverse** (`Sum → -`, `BitXor` self-inverse), executed through the
  driver path like the forward merge.
- **End-state equivalence (DuckDB harness) — must not regress + extended.** Real-fixture e2e: reprocessing a
  **changed** partition for a `SUM`/`COUNT` model converges to a full refresh over the corrected inputs
  **without** a full rebuild; the forward (non-reprocessing) rung-1 cumulative equivalence cases stay green.
  Add under `crates/smelt-cli/tests/e2e/` (sibling of `backbuild_cumulative_e2e.rs`). Requires
  `DUCKDB_LIB_DIR`.
- **Fail-closed reject test (non-group must still refuse).** A `MIN` (or `MAX`) model asked to reprocess an
  already-merged partition still refuses with the §"Reprocessing semantics" diagnostic pointing at
  `--full-refresh` (you cannot un-see a maximum from stored state). This is the required non-group retraction
  refusal.
- `--auto` staleness — for a fully-reversible model `--auto` returns **exactly the changed source
  partitions**; for a non-reversible model it returns the conservative "any partition ≥ earliest stale;
  force full refresh if earlier partitions are stale" (spec §"Interaction with `--auto` / staleness").

**Implementation shape.** Add a per-partition **delta side table** `<model>__deltas(partition_value, <state
cols>)` written on each merge step, kept for reversible models (see §"Open decisions" on always-keep vs
opt-in). Add an **inverse** rendering to `CrossPartitionCombiner` (`cumulative.rs:43`) for the group subset
(`Sum` → subtraction; `BitXor` → itself); non-group variants have no inverse — the classifier marks the
model non-reversible and the reprocess path refuses. Reprocessing path in `runtime/src/cumulative.rs`: for a
changed partition, apply the stored delta under the inverse (subtract), recompute the partition's delta,
then apply it normally (add), through the F11 driver. Wire `--auto` staleness to consult reversibility + the
delta side table's covered partitions.

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — reversibility classification (reads F4); inverse rendering
  on `CrossPartitionCombiner` (`:43`).
- `crates/smelt-runtime/src/cumulative.rs` — delta side-table write, subtract-then-add reprocess,
  `execute_cumulative_aggregate` (`:34`); `--auto` staleness path.
- `crates/smelt-cli/tests/e2e/backbuild_cumulative_e2e.rs` (+ sibling reprocess test).

**Docs touched.**
- `cumulative_aggregate.md` — **remove/narrow** the §Known-Divergence notes "Reprocessing via delta history"
  and "`--auto` staleness fidelity" for the reversible subset (they become behaviour); keep the non-group
  reprocessing refusal in §"Reprocessing semantics"; narrow the rung note to rung 4 outstanding. The
  reversible-vs-irreversible split in §"Interaction with `--auto`" is already normative — align it with the
  shipped behaviour.
- `model_transforms.md` §Known Divergences — remove "retraction via delta history" from the "Unbuilt" list.
- `docs-site/docs/guide/materializations.md` — note reprocessing is supported for the reversible aggregators.

**Review checklist.**
- [ ] Reversibility derived from the projection (F4); a `MIN`/`MAX` reprocess still refuses (`--full-refresh`).
- [ ] Subtract-then-add reprocessing of a changed partition equals full refresh, no rebuild.
- [ ] `--auto` returns exactly the changed partitions for fully-reversible models; conservative otherwise.
- [ ] Delta side table is never a downstream dependency target; **forward rung-1 equivalence unchanged.**
- [ ] Spec/docs edits timeless; reversible-subset Known-Divergence notes removed.

**Commit.** `feat(cumulative): rung-3 group retraction via delta history + --auto fidelity`

---

### Phase CU4: Rung 4 — opt-in bounded-domain multiset for exact holistic aggregates

**Goal.** Admit exact `MEDIAN`/`PERCENTILE`/`MODE`/quantiles/exact-`COUNT(DISTINCT)`/`DISTINCT`-aggregates by
driving the **explicit bounded-domain multiset state** transform — store the per-key `value → count`
multiset (a bounded-domain Z-set) — **gated on the bounded-domain-budget declaration owned by the L3
declarations sub-plan** (`docs/plans/20260704-model-updates-l3-declarations.md`), with a runtime cap that
falls back to full refresh; **default-refuse** unbounded state with a fail-loud message suggesting the
approximate form (CU2's sketch) or `refresh: full`. Per `cumulative_aggregate.md` §"The maintenance boundary"
(rung 4) and research §14.4.

**Spec anchor.** `cumulative_aggregate.md` §"The maintenance boundary" (rung 4 — opt-in, fail-loud
bounded-domain multiset); `model_maintenance.md` ladder rung 4 (opt-in bounded-domain multiset; Z-set;
`O(active domain)` state, default-refused without a budget, runtime-capped); `model_transforms.md` "Explicit
bounded-domain multiset state" + §Constraints "The bounded-domain multiset is opt-in and capped". Consumed by
name: F4 holistic discriminant; the **bounded-domain-budget declaration** (L3 sub-plan).

**Pre-conditions.** CU1 landed — the multiset state table + `π` presentation reuse CU1's atomic `(state
table, view)` unit (the multiset is a decomposed state; `π` is any distribution functional). The
**bounded-domain-budget declaration surface** is landed by the L3 declarations sub-plan; CU4 reads it as the
opt-in licence. If that surface is not yet built, **block CU4** (do not author the declaration here — it is
L3's).

**Depends on.** CU1, F4, L3 bounded-domain-budget declaration
(`docs/plans/20260704-model-updates-l3-declarations.md`).

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/cumulative.rs` unit — **without** the declared budget, `MEDIAN(x)` refuses
  with a fail-loud message naming the approximate form or `refresh: full`; **with** the bounded-domain budget
  declared (L3 surface), `MEDIAN(x)` classifies (via F4's holistic discriminant) to a `value → count`
  multiset state with `π` = median. Same for exact `COUNT(DISTINCT)` and `MODE`.
- `crates/smelt-runtime/src/cumulative.rs` unit — the multiset state merges by **componentwise count
  addition**; one multiset state serves multiple `π` (median, mode, exact-distinct) off the same table.
- **End-state equivalence (DuckDB harness) — must not regress + extended.** Real-fixture e2e: exact `MEDIAN`
  with a declared bounded domain has end-state equality with a full refresh across ≥3 partitions; CU1/CU2/CU3
  and rung-1 cases stay green. Add under `crates/smelt-cli/tests/e2e/`. Requires `DUCKDB_LIB_DIR`.
- **Fail-closed reject test (two forms).** (a) **Default-refuse**: an unbounded holistic aggregate **without**
  the L3 budget declaration refuses (names the approximate form / `refresh: full`). (b) **Runtime cap**: a
  model that **exceeds** its declared domain cap **falls back to full refresh** (observable: warning +
  rebuild), never silent corruption or unbounded growth.

**Implementation shape.** Add a **multiset state** to CU1's decomposition machinery: state table `<model>__
state` stores `(<keys>, value, count)`; the presentation view computes the requested functional (`MEDIAN`,
`MODE`, exact distinct) via `π`. One state, many presentations (research §14.4). The classifier reads the
**L3 bounded-domain-budget declaration** as the opt-in licence; absent it, the holistic aggregate refuses
(default-refuse). Add a **runtime cap** check on multiset cardinality with a **full-refresh fallback** when
exceeded. The opt-in **must be a space assertion, not a contract-changing strategy knob** — the SQL still
just says `MEDIAN` (research §18.2).

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — the multiset decomposition + budget-declaration read +
  default-refuse (reads F4 + the L3 surface).
- `crates/smelt-runtime/src/cumulative.rs` — multiset merge, cap check, full-refresh fallback.
- `crates/smelt-cli/tests/e2e/` — exact-`MEDIAN` equivalence + cap-fallback + default-refuse.
- `examples/` — a bounded-domain `MEDIAN` fixture.

**Docs touched.**
- `cumulative_aggregate.md` — **spec increment (pre-authorised)**: add the exact holistic aggregators to
  §Surface "Aggregator allowlist" **behind the L3 bounded-domain budget** (cite the declaration surface,
  which the L3 sub-plan authors); **remove** the rung-4 §Known-Divergence note. Do **not** author the
  declaration surface itself here — reference the L3 sub-plan's spec increment.
- `model_transforms.md` §Known Divergences — remove "bounded-domain multiset" from the "Unbuilt" list.
- `docs-site/docs/guide/materializations.md` — document the exact holistic aggregators and the
  default-refuse behaviour (timeless), linking the budget declaration.

**Review checklist.**
- [ ] Exact `MEDIAN` with a declared bounded domain equals full refresh; the opt-in is a space assertion only.
- [ ] Unbounded state without the L3 budget refuses (fail-loud, names the approximate form / `refresh: full`).
- [ ] Exceeding the runtime cap falls back to full refresh, never silent.
- [ ] The multiset reuses CU1's `(state table, view)` unit; state table not a downstream target; **CU1–CU3 +
      rung-1 equivalence unchanged.**
- [ ] Spec/docs edits timeless; the holistic aggregators added to §Surface behind the L3 budget, rung-4
      Known-Divergence removed; the budget surface is L3's, referenced not re-authored.

**Commit.** `feat(cumulative): rung-4 opt-in bounded-domain multiset for exact holistic aggregates`

---

## Blocked phases

(none yet)

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this sub-plan.)

- General-operator retraction over joins; unbounded exact-distinct the user cannot cap — delegated to
  native IVM via `refresh: materialized_view` (its own L4 sub-plan). The sibling keyed modes
  (`latest_value` / `versioned` / `accumulating_snapshot`) are separate L4 sub-plans reusing F11 + F12.

## Open decisions surfaced for the implementer

Genuine design choices the master (research §18.2) leaves to the sub-plan. Settle each in the owning phase;
if a choice cannot be made from the spec + research, **block** the phase (do not guess a contract-changing
default).

- **Closed decomposition table vs registry (CU2).** Settled here as a **closed, hard-coded rewrite table**
  beside `combiner_for` (`cumulative.rs:82`) — matches cumulative's "fixed allowlist, not a registry" stance
  (`cumulative_aggregate.md` §Design; research §18.2). Revisit only on a concrete custom-sketch motivator.
- **Approximate-distinct sketch representation (CU2).** Which DuckDB-available state backs
  `APPROX_COUNT_DISTINCT` register-wise merge (HLL via the `hll` extension, or an approx-aggregation state
  that round-trips through the state table), and the documented error tolerance for the equivalence harness.
- **Delta-history side table: always-keep vs opt-in (CU3).** Keep `<model>__deltas` for every reversible
  model (space cost, simplest), or only when reprocessing is expected. Default to always-keep for reversible
  models; note the space cost.
- **Bounded-domain budget declaration shape (CU4).** This is **owned by the L3 declarations sub-plan**
  (`docs/plans/20260704-model-updates-l3-declarations.md`), which authors the surface as a **space
  assertion, not a strategy knob** (research §18.2). CU4 only *reads* it. If the L3 surface is not yet
  landed when CU4 runs, **block CU4** — do not author the declaration here.
- **Exact vs approximate selection (CU4 vs CU2).** Exact holistic aggregates go through CU4's multiset;
  approximate forms (t-digest / HLL) are CU2 decomposed monoids needing no budget. Keep the
  exact-vs-approximate choice legible to the modeller (which operator they write), not a hidden fidelity knob.

## Verification

How to confirm the L4 cumulative composition is satisfied at the end:
- `cargo test` (workspace) green; `cargo clippy --all-targets` clean; `cargo fmt --all -- --check`.
- **Per-rung end-state-equivalence harness** green: `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs`
  and `crates/smelt-cli/tests/cli_unit/cumulative_equivalence.rs` extended to `AVG` (CU1),
  variance/approx-distinct (CU2), reprocessing (CU3), and exact `MEDIAN` (CU4) — each equals a full refresh
  over the processed inputs (`cumulative_aggregate.md` §"Cross-partition equivalence"). Requires
  `DUCKDB_LIB_DIR`.
- **No regression of the cumulative end-state equivalence harness.** The existing rung-1 cumulative
  integration/equivalence tests (`crates/smelt-cli/tests/cumulative*`) stay green after every phase — the
  end-state equivalence oracle is the net (`model_maintenance.md` §"The equivalence invariant"). A phase
  that flips one is a wiring bug, not a spec change.
- **Per-rung fail-closed reject test** green: `π`-purity refusal (CU1), exact-distinct/`STRING_AGG` refusal
  (CU2), non-group (`MIN`/`MAX`) reprocess refusal (CU3), unbounded-without-budget refusal + cap fallback
  (CU4). An aggregate off its rung, or unbounded multiset state without a declared budget, is **refused**
  with a diagnostic suggesting the approximate form or `refresh: full` — never applied approximately
  (`model_transforms.md` §Constraints "Equivalence or refusal").
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces`
  green — the new `examples/` cumulative fixtures build with zero diagnostics.
- `/smelt:validate cumulative_aggregate`, `/smelt:validate model_transforms`, `/smelt:validate
  model_maintenance` report zero drift for the rungs this layer lands: every rung-2/3/4 §Known-Divergence
  note in `cumulative_aggregate.md` is removed (or narrowed to the outstanding rung), and the
  decomposed-state-plus-view / retraction / bounded-domain-multiset entries in `model_transforms.md` §Known
  Divergences "Unbuilt" list are gone as their phase lands.
