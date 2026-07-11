# Plan: Model updates — Group C (Keyed-mode maintenance rungs — cumulative extensions)

**Date**: 2026-07-04
**Master plan**: [`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) — Group C (phases C1–C4)
**Specs (oracles)**:
- [`docs/specs/cumulative_aggregate.md`](../specs/cumulative_aggregate.md) — **primary**. §"The maintenance boundary (algebraic ladder)" (rungs 1–4), §"Reprocessing semantics", §"Interaction with `--auto` / staleness", §"Cross-partition equivalence", §"Output shape", §Surface (aggregator allowlist + diagnostic codes).
- [`docs/specs/models.md`](../specs/models.md) — §"Refresh axis" (cumulative is a keyed-output peer on the refresh axis), §"Constraint violations".
**Research (design, not scope)**: [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — Part 13 (§13.1 direct vs hidden state), Part 14 (the algebraic ladder — §14.1 decomposed monoid, §14.2 group/retraction, §14.4 opt-in bounded-domain multiset), Part 15 (§15.1 `(state table + view)` = Enzyme's trick portably; §15.3 the two emulation hazards — presentation-view purity + atomic state/view swap), §18.2 (the two open decisions: closed-table vs registry for decomposition; the bounded-domain opt-in surface shape).
**Spec diff**: the 2026-07-04 spec edits (committed in `f056ac35`) that added the Part-14 algebraic ladder (rungs 1–4) to `cumulative_aggregate.md` §"The maintenance boundary" and the rung-2/3/4 §Known-Divergence notes. This plan builds rungs 2–4 (rung 1 — the direct-monoid allowlist — already ships), closing the code↔spec gap those edits opened.
**Tracking branch**: `worktree-incremental`
**Docs**: code+docs

**Enabling-phase decision (fixed by the master, do not re-open).** C1 introduces the **`(state table, presentation view)` as one atomically-swapped unit** machinery (research §15.3). C2 and C4 reuse it verbatim — they add new decomposition/multiset states behind the same view mechanism, never a second view path. C3 is the group-retraction rung and needs no presentation view (its combiners are direct monoids that also happen to be groups), so it depends only on A1, not on C1.

---

## Execution prompt (for a fresh Claude session / the autonomy loop)

You are executing this plan phase by phase. It is a sub-plan registered in
[`docs/plans/20260704-model-updates.md`](20260704-model-updates.md) §"Spawned sub-plans".

**Before touching any code:**
1. Read this entire plan, then read the cited `cumulative_aggregate.md` sections — they are the correctness oracle. The **maintained-relation equivalence contract holds unconditionally for every rung** (spec §"The maintenance boundary"); what changes across rungs is the *state representation and its size*, never the fidelity of the user-visible value. Do not weaken cross-partition equivalence to make a rung land.
2. Confirm you are on branch `worktree-incremental` and that Group A (esp. A1 — the `refresh:` axis + `RefreshStrategy::Cumulative`) has landed. Group C depends on A1.
3. Find the next `pending` row in the Progress-tracking table below. That is your phase. If every row is `done`, run §Verification, flip this sub-plan's registry Status to `done` in the master, and stop.

**Per phase, run `/smelt:implement`'s loop:** pre-flight (`cargo build`/`cargo test` green except this phase's own red target) → implementer subagent (red-green TDD on the listed tests, real fixtures in `examples/`) → reviewer subagent (material findings only) → iterate → set the row `done` → commit + push with the phase's `Commit.` line. A phase's row lists a **spec increment** where one is pre-authorised; making the cited edits is expected, not scope creep.

**Ordering.** C1 → (C2 ∥ C3 ∥ C4). C1 is the enabling phase and must land first. C2 and C4 depend on C1's presentation-view unit. C3 depends only on A1 and may run any time after A1 — sequence it after C1 only to keep commits clean.

**The oracle for every rung.** Each phase ships an **end-state-equivalence harness** (`cumulative_run(π(S)) == full_refresh(source.where(partition ∈ S))`, spec §"Cross-partition equivalence") extended to the rung's new aggregators, **and** a **fail-loud test** (unbounded state refused, or a non-group aggregator that must still refuse under retraction). Both are required by the master's §"Post-implementation verification (per group)" C row.

**Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this file only*. Spec + `docs-site/` edits describe the feature as if it always existed; as each rung lands, **remove** the matching §Known-Divergence note (or narrow it to the rungs still outstanding) rather than annotating it with a phase number.

**Block rule.** On a design decision not answered here or by the spec (e.g. the C4 opt-in surface shape if you cannot settle it), or a pre-flight red unrelated to this phase's target: set the row `blocked` with a one-line reason, append to §"Blocked phases", restore a clean tree, commit, emit `<<PHASE_BLOCKED>>`. Otherwise emit `<<PHASE_COMPLETE>>`.

---

## Context

Cumulative today implements exactly **rung 1** of the algebraic ladder: the direct-monoid allowlist
(`SUM`/`COUNT`/`MIN`/`MAX`/`BOOL_*`/`BIT_*`), where the stored column *is* the answer and the combiner
is a directly-presentable commutative monoid. The classifier lives in
`crates/smelt-logical/src/rules/cumulative.rs` (`classify_cumulative` `:227`, `combiner_for` `:82`,
`CrossPartitionCombiner` `:43`, `AggregatorColumn` `:30`, `CumulativeClassification` `:183`; the
`smelt-planner` copy at `crates/smelt-planner/src/rules/cumulative.rs:1` is a one-line re-export). The
per-partition merge loop lives in `crates/smelt-runtime/src/cumulative.rs` (`execute_cumulative_aggregate`
`:34`, partition loop `:117`, first-run `create_table_as` `:155`, `build_cumulative_merge_sql` `:218`,
dispatched from `crates/smelt-runtime/src/execute.rs:779`). The loop executes a **raw MERGE via
`backend.execute_sql(...)`** — it does not call `merge_into`. There is **no** presentation-view machinery
yet — the target table *is* the user-facing relation. (A generic `create_view_as` exists on the backend
trait — `crates/smelt-backend/src/lib.rs:37`, DuckDB impl `crates/smelt-backend-duckdb/src/lib.rs:320` —
but nothing on the cumulative path uses it.)

Group C climbs the remaining three rungs, all specified ahead of implementation in
`cumulative_aggregate.md` §"The maintenance boundary" and derived in research Part 14:

- **Rung 2 (decomposed monoid, C1+C2).** The user value is `π(state)` where `state` is a monoid element
  in a richer space and `π` a presentation map. `AVG` = `(sum, count)` presented `sum/count`; variance =
  a Welford triple; approximate distinct = an HLL register vector. smelt keeps the intermediate in an
  ordinary **state table** and exposes the user value through a **presentation view** — the portable
  re-implementation of the engine hidden-state trick (research §15.1).
- **Rung 3 (group / retraction, C3).** When inputs change, the combiner must be *invertible*: `SUM`,
  `COUNT`, `BIT_XOR` are groups (`x ⊕ y ⊖ y = x`) and admit subtract-then-add reprocessing from a
  per-partition delta history; `MIN`/`MAX`/`BOOL_*`/`BIT_AND`/`BIT_OR` are monoids-not-groups and still
  require a full refresh to reprocess.
- **Rung 4 (opt-in bounded-domain multiset, C4).** Exact holistic aggregates (`MEDIAN`, quantiles,
  `MODE`, exact `COUNT(DISTINCT)`, `DISTINCT`-aggregates) are maintainable by storing the per-key
  `value → count` multiset (a bounded-domain Z-set), **only** behind a fail-loud space-budget opt-in.

The maintained-relation equivalence contract (spec §"Cross-partition equivalence") holds unconditionally
across all four rungs; Group C never trades correctness for a rung, only state size.

## Scope

### In scope
- **C1** — the decomposed-monoid *mechanism*: `AVG → (sum,count)` state table + presentation view;
  `(state table, view)` as one atomically-swapped unit; the `π`-purity contract (research §15.3).
- **C2** — additional decomposed states over C1's mechanism: variance/stddev (Welford triple) and
  approximate `COUNT(DISTINCT)` (HLL/sketch register vector), each with its presentation map.
- **C3** — the group rung: per-partition delta history for the reversible subset (`SUM`/`COUNT`/`BIT_XOR`),
  subtract-then-add reprocessing, reversibility derived from the projection, and `--auto` staleness
  upgraded to "exactly the changed partitions" for fully-reversible models.
- **C4** — the opt-in bounded-domain multiset rung for exact holistic aggregates, with a runtime cap +
  full-refresh fallback and a default-refuse fail-loud message.

### Explicitly deferred
- **General-operator retraction over joins, and `DISTINCT`/exact-distinct whose state is unbounded in a
  dimension the user cannot cap** — not smelt-driven-maintainable; delegated to native IVM via
  `refresh: materialized_view` (Group D / `materialized_view.md`). C4 moves the boundary only for
  *single-column* holistic aggregates the user can bound (research §14.3–§14.4).
- **The new keyed modes** `versioned` / `latest_value` (Group D) — they reuse C1's presentation-view
  plumbing but are separate classifiers, not cumulative rungs.
- **A decomposition *registry*.** The decomposed-monoid set stays a **closed, hard-coded rewrite table**
  beside `combiner_for` (`cumulative.rs:82`), matching cumulative's "fixed allowlist, not a registry"
  stance (`cumulative_aggregate.md` §Design). Revisit only on a concrete custom-sketch motivator (§18.2).

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| C1    | pending |        |      |
| C2    | pending |        |      |
| C3    | pending |        |      |
| C4    | pending |        |      |

---

### Phase C1: Decomposed-monoid rung — `AVG` via `(sum,count)` state table + presentation view

**Goal.** Admit `AVG` by storing `(sum, count)` in a **state table** under componentwise `+` and exposing
`sum/count` through a **presentation view**, treating `(state table, view)` as one atomically-swapped
unit. This is the **enabling mechanism** for the whole rung-2 unlock (C2 reuses it) — the load-bearing
deliverable of Group C. Per `cumulative_aggregate.md` §"The maintenance boundary" (rung 2) and research
§13.1 / §14.1 / §15.1 / §15.3.

**Pre-conditions.** A1 landed (`RefreshStrategy::Cumulative` at `config.rs:31`; the classifier
`classify_cumulative` at `cumulative.rs:227`; `combiner_for` allowlist at `cumulative.rs:82`; the merge
loop `execute_cumulative_aggregate` at `runtime/src/cumulative.rs:34`). No presentation-view machinery
exists today — the target table *is* the user relation.

> **Scope note.** C1 ships **only `AVG`** as the first decomposed aggregator. Variance/stddev and
> approximate-distinct are C2 — they are additional entries in the same decomposition table, added once the
> mechanism this phase builds is proven on `AVG`. Do not add them here.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/cumulative.rs` unit — `AVG(amount)` in a non-key projection now
  **classifies** (today it refuses `CumulativeUnknownAggregator`): the resulting `AggregatorColumn`
  (`cumulative.rs:30`) is a *decomposed* variant carrying state columns `(_sum_amount, _count_amount)`,
  their componentwise `Sum` combiners, and a presentation expression `_sum_amount / _count_amount AS
  avg_amount`. A composite `AVG(x) + 1` still refuses (`CumulativeUnknownAggregator`, not a direct call).
- `crates/smelt-logical/src/rules/cumulative.rs` unit — the decomposition table is the single source of
  truth (parallel to `combiner_for` at `:82`); a still-unknown aggregator (`STRING_AGG`) refuses.
- `crates/smelt-runtime/src/cumulative.rs` unit — `build_cumulative_merge_sql` (`:218`) for an `AVG`
  model merges **both** `_sum` and `_count` state columns componentwise; a new presentation-view builder
  emits `CREATE OR REPLACE VIEW <model> AS SELECT <keys>, _sum/_count AS avg_… FROM <model>__state`.
- `crates/smelt-runtime/src/cumulative.rs` unit / integration — the state table and the view are
  created/replaced as **one atomically-swapped unit** (research §15.3 hazard 2): the first-run path
  (`create_table_as` at `:155`) creates the state table then the view in the same step; the view is never
  left pointing at a stale/absent state table.
- Real-fixture e2e (examples/) — a cumulative model with an `AVG` column maintained across ≥3 source
  partitions has **end-state equality with a full refresh** over the union of those partitions. Add the
  fixture (e.g. an `avg_ticket` column on `examples/web_analytics/models/silver/device_user_edges.sql`,
  or a dedicated `examples/cumulative_avg/`) and the assertion under
  `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs` (or a sibling
  `crates/smelt-cli/tests/cli_unit/cumulative_equivalence.rs` case).
- Fail-loud (`π`-purity, research §15.3 hazard 1) — the classifier only admits decompositions whose `π`
  is a pure function of a **single** state row; a hypothetical decomposition whose presentation would
  reference another row/table is rejected (assert the admitted set is exactly the closed table, no
  cross-row `π`).

**Implementation shape.**
- Widen `AggregatorColumn` (`cumulative.rs:30`) so a column is either **direct** (today's
  `CrossPartitionCombiner` over the user column, `:43`) or **decomposed**: a set of hidden state columns,
  each with its own per-partition aggregator + componentwise combiner, plus a presentation SQL expression.
- Add a **closed decomposition table** beside `combiner_for` (`:82`) mapping `AVG(x)` →
  `{state: [SUM(x) AS _sum_x (combiner Sum), COUNT(x) AS _count_x (combiner Sum)], π: "_sum_x / _count_x"}`.
- `CumulativeClassification` (`cumulative.rs:183`) now carries the **state-table projection** (the
  per-partition delta SELECT selects state columns) and a **presentation-view definition**.
- Execution (`execute_cumulative_aggregate`, `runtime/src/cumulative.rs:34`): the physical merge target is
  the **state table** (`<model>__state`, hidden columns); after the merge loop, (re)create the presentation
  **view** named `<model>` as the user-facing relation, atomically with the state table. `build_cumulative_merge_sql`
  (`:218`) merges state columns componentwise and is run via `backend.execute_sql(...)` (the loop executes a
  raw MERGE, it does not call `merge_into`); first-run `create_table_as` (`:155`) creates the state table.
  Reuse the **already-present but currently-unused** `create_view_as` primitive (trait
  `crates/smelt-backend/src/lib.rs:37`; DuckDB impl `crates/smelt-backend-duckdb/src/lib.rs:320`, emitting
  `CREATE VIEW … AS …`) for the presentation view — no new backend method is needed.
- Downstream refs resolve to the **view** — `cumulative_aggregate.md` §"Output shape" (a keyed lookup,
  no partition column) is unchanged; the state table is never a dependency target (research §18.2).

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — `AggregatorColumn` (`:30`), `CrossPartitionCombiner`
  (`:43`), `combiner_for` (`:82`), `CumulativeDiagnostic` (`:100`), `CumulativeClassification` (`:183`),
  `classify_cumulative` (`:227`).
- `crates/smelt-runtime/src/cumulative.rs` — `execute_cumulative_aggregate` (`:34`), the partition loop
  (`:117`), first-run `create_table_as` (`:155`), `build_cumulative_merge_sql` (`:218`); dispatched from
  `crates/smelt-runtime/src/execute.rs:779`.
- `crates/smelt-backend/src/lib.rs:37` (`create_view_as` trait) / `crates/smelt-backend-duckdb/src/lib.rs:320`
  (DuckDB impl, unused today) — reuse for the presentation view. The merge itself is raw SQL via
  `execute_sql`; `merge_into` (`smelt-backend/src/lib.rs:286`) is not on the cumulative path.
- `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs`,
  `crates/smelt-cli/tests/cli_unit/cumulative_equivalence.rs` — extend for `AVG`.
- `examples/web_analytics/` (or new `examples/cumulative_avg/`) — the real fixture.

**Docs touched.**
- `docs/specs/cumulative_aggregate.md` — **spec increment (pre-authorised)**: add `AVG` to the §Surface
  "Aggregator allowlist" as a *decomposed* aggregator (state `(sum,count)`, presented `sum/count`); the
  rung-2 mechanism it lands is already normative in §"The maintenance boundary". **Remove** the §Known-Divergence
  "`AVG` rewrite (rung 2)" note and **narrow** the "Only the direct-monoid rung is implemented" note to
  rungs 3–4 outstanding.
- `docs-site/docs/guide/materializations.md` — note `AVG` is available in cumulative models (timeless prose;
  no phase vocabulary, no "now supports").

**Review checklist.**
- [ ] `AVG` classifies to a decomposed state; `AVG(x)+1` and `STRING_AGG` still refuse.
- [ ] State table + presentation view move as one atomically-swapped unit; downstream sees only the view.
- [ ] End-state equivalence harness extended to `AVG` and green; the `π`-purity fail-loud test refuses a cross-row presentation.
- [ ] Decomposition table is a single closed source of truth (no second place to edit).
- [ ] Spec/docs edits are timeless; the `AVG` Known-Divergence note is removed, the rung note narrowed.

**Commit.** `feat(cumulative): AVG via decomposed (sum,count) state table + presentation view`

---

### Phase C2: Decomposed-monoid rung — variance/stddev (Welford) + approximate distinct

**Goal.** Add the Welford-triple state for `VAR`/`STDDEV` and an HLL/sketch register-vector state for
approximate `COUNT(DISTINCT)`, each with its presentation map, **reusing C1's `(state table, view)`
mechanism**. Per `cumulative_aggregate.md` §"The maintenance boundary" (rung 2) and research §14.1.

**Pre-conditions.** C1 landed (the decomposed-`AggregatorColumn` variant, the closed decomposition table,
and the atomic state-table/view execution path all exist and are proven on `AVG`).

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/cumulative.rs` unit — `VAR_POP(x)` / `STDDEV(x)` classify to a
  `(count, sum, sum_of_squares)` (or numerically-stable Welford) triple state with a closed-form `π`;
  `APPROX_COUNT_DISTINCT(x)` classifies to a sketch register-vector state with `π` = cardinality estimate.
- `crates/smelt-runtime/src/cumulative.rs` unit — the variance triple merges componentwise; the sketch
  state merges register-wise (`max` / HLL merge) in `build_cumulative_merge_sql`.
- Real-fixture e2e (examples/) — a `VAR`/`STDDEV` cumulative model has **exact** end-state equality with a
  full refresh (variance is exact under componentwise merge); an `APPROX_COUNT_DISTINCT` model is within a
  documented relative-error tolerance of the exact full-refresh distinct count. Extend
  `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs`.
- Fail-loud — **exact** `COUNT(DISTINCT)` still refuses at C2 (it is the C4 opt-in multiset, not a bounded
  decomposed monoid); `STRING_AGG` still refuses. The message points at the approximate form or `refresh: full`.

**Implementation shape.**
- Extend C1's closed decomposition table with `VAR_*`/`STDDEV_*` → Welford triple and
  `APPROX_COUNT_DISTINCT` → sketch register vector; no new execution path — the merge loop, state table,
  and presentation view are C1's.
- The **closed-table vs registry** decision (research §18.2) is settled here in favour of the **closed
  table** (matches cumulative's fixed-allowlist stance). Do not add a registry surface.
- Sketch representation: pick the DuckDB-available approximate-distinct state (see Open decisions below);
  keep the choice inside the decomposition table so the classifier stays derive-from-SQL.

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — the decomposition table + `AggregatorColumn` variants.
- `crates/smelt-runtime/src/cumulative.rs` — componentwise / register-wise merge in `build_cumulative_merge_sql`.
- `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs` — variance + approx-distinct coverage.
- `examples/` — a fixture exercising `VAR`/`STDDEV`/`APPROX_COUNT_DISTINCT`.

**Docs touched.**
- `docs/specs/cumulative_aggregate.md` — **spec increment (pre-authorised)**: add `VAR`/`STDDEV`/
  `APPROX_COUNT_DISTINCT` to §Surface "Aggregator allowlist" as decomposed aggregators; further narrow the
  "Only the direct-monoid rung is implemented" Known-Divergence note toward rungs 3–4.
- `docs-site/docs/guide/materializations.md` — list the new decomposed aggregators (timeless).

**Review checklist.**
- [ ] Variance is exact vs full refresh; approx-distinct within the documented tolerance.
- [ ] Exact `COUNT(DISTINCT)` still refuses (deferred to C4); `STRING_AGG` still refuses.
- [ ] No new execution path — C1's state-table/view mechanism is reused unchanged.
- [ ] Decomposition stays a closed table (no registry); the sketch choice is documented.
- [ ] Spec/docs edits timeless.

**Commit.** `feat(cumulative): variance/stddev Welford triple + approximate-distinct sketch states`

---

### Phase C3: Group rung — retraction via per-partition delta history + `--auto` fidelity

**Goal.** For the **group** (reversible) aggregators `SUM`/`COUNT`/`BIT_XOR`, store per-partition deltas
in a side table so a changed source partition is reprocessed by **subtract-then-add**; derive reversibility
from the projection; upgrade `--auto` staleness to "exactly the changed partitions" for fully-reversible
models. `MIN`/`MAX`/`BOOL_*`/`BIT_AND`/`BIT_OR` are monoids-not-groups and still require a full refresh to
reprocess. Per `cumulative_aggregate.md` §"Reprocessing semantics", §"Interaction with `--auto` / staleness",
§"The maintenance boundary" (rung 3), and research §14.2.

**Pre-conditions.** A1 landed. C3 depends only on A1 — it operates on **direct-monoid** columns that are
also groups, so it needs no presentation view (independent of C1). Today reprocessing is refused at
planning time (`cumulative_aggregate.md` §"Reprocessing semantics"; the metadata validation and refusal
live around `crates/smelt-core/src/metadata.rs` + the runtime loop) and `--auto` is conservative.

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/cumulative.rs` unit — a model whose every non-key column combiner is in
  the group subset `{Sum, BitXor}` (i.e. `SUM`/`COUNT`/`BIT_XOR`) classifies **reversible**; a model
  containing `Min`/`Max`/`BoolAnd`/`BoolOr`/`BitAnd`/`BitOr` classifies **non-reversible**. Reversibility is
  *derived from the projection*, never declared.
- `crates/smelt-runtime/src/cumulative.rs` unit — after merging partition `D`, its delta is retained in the
  side table `<model>__deltas`; the reprocessing builder emits subtract (stored delta) then add (recomputed
  delta) using each group combiner's **inverse** (`Sum → -`, `BitXor` self-inverse), executed via
  `execute_sql` like the forward MERGE.
- Real-fixture e2e (examples/) — reprocessing a **changed** partition for a `SUM`/`COUNT` model converges
  to a full refresh over the corrected inputs, **without** a full rebuild. Add under
  `crates/smelt-cli/tests/e2e/` (sibling of `backbuild_cumulative_e2e.rs`).
- Fail-loud (non-group must still refuse) — a `MIN` (or `MAX`) model asked to reprocess an already-merged
  partition still refuses with the §"Reprocessing semantics" diagnostic pointing at `--full-refresh` (you
  cannot un-see a maximum from stored state). This is the master's required non-group retraction fail-loud.
- `--auto` staleness — for a fully-reversible model `--auto` returns **exactly the changed source
  partitions**; for a non-reversible model it returns the conservative "any partition ≥ earliest stale;
  force full refresh if earlier partitions are stale" (spec §"Interaction with `--auto` / staleness").

**Implementation shape.**
- Add a per-partition **delta side table** `<model>__deltas(partition_value, <state cols>)` written on each
  merge step; keep it only for reversible models (see Open decisions on always-keep vs opt-in).
- Add an **inverse** rendering to `CrossPartitionCombiner` (`cumulative.rs:43`) for the group subset
  (`Sum` → subtraction; `BitXor` → itself). Non-group variants have no inverse — the classifier marks the
  model non-reversible and the reprocess path refuses.
- Reprocessing path in `runtime/src/cumulative.rs`: for a changed partition, apply the stored delta under
  the inverse combiner (subtract), recompute the partition's delta, then apply it normally (add) — both via
  `execute_sql` raw MERGE, matching the forward loop.
- Wire `--auto` staleness analysis to consult reversibility + the delta side table's covered partitions.

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — reversibility classification; `CrossPartitionCombiner`
  inverse (`:43`).
- `crates/smelt-runtime/src/cumulative.rs` — delta side table write, subtract-then-add reprocess,
  `execute_cumulative_aggregate` (`:34`); `--auto` staleness path.
- `crates/smelt-cli/tests/e2e/backbuild_cumulative_e2e.rs` (+ sibling reprocess test).

**Docs touched.**
- `docs/specs/cumulative_aggregate.md` — **remove/narrow** the Known-Divergence notes "Reprocessing via
  delta history" and "`--auto` staleness fidelity" for the reversible subset (they become behaviour, not
  divergence); keep the non-group reprocessing refusal in §"Reprocessing semantics"; narrow the rung note to
  rung 4 outstanding. The reversible-vs-irreversible split in §"Interaction with `--auto`" is already
  normative — align it with the shipped behaviour.
- `docs-site/docs/guide/materializations.md` — note reprocessing is supported for the reversible aggregators.

**Review checklist.**
- [ ] Reversibility is derived from the projection; a `MIN`/`MAX` model reprocess still refuses (`--full-refresh`).
- [ ] Subtract-then-add reprocessing of a changed partition equals full refresh, no rebuild.
- [ ] `--auto` returns exactly the changed partitions for fully-reversible models; conservative otherwise.
- [ ] Delta side table is never a downstream dependency target.
- [ ] Spec/docs edits timeless; reversible-subset Known-Divergence notes removed.

**Commit.** `feat(cumulative): group-rung retraction via per-partition delta history + --auto fidelity`

---

### Phase C4: Opt-in bounded-domain multiset — exact holistic aggregates

**Goal.** Admit exact `MEDIAN`/`PERCENTILE`/`MODE`/quantiles/exact-`COUNT(DISTINCT)`/`DISTINCT`-aggregates
by storing the per-key **value → count multiset** (a bounded-domain Z-set), **only** behind a
bounded-domain space-budget **opt-in** with a runtime cap that falls back to full refresh; **default-refuse**
unbounded state with a fail-loud message suggesting the approximate form (C2's sketch) or `refresh: full`.
Per `cumulative_aggregate.md` §"The maintenance boundary" (rung 4) and research §14.4.

**Pre-conditions.** C1 landed — the multiset state table + `π` presentation reuse C1's atomic
`(state table, view)` unit (the multiset is a decomposed state; `π` is any distribution functional).

**TDD tests to write first.**
- `crates/smelt-logical/src/rules/cumulative.rs` unit — **without** the opt-in, `MEDIAN(x)` refuses with a
  fail-loud message naming the approximate form or `refresh: full`; **with** the bounded-domain opt-in
  declared, `MEDIAN(x)` classifies to a `value → count` multiset state with `π` = median. Same for
  exact `COUNT(DISTINCT)` and `MODE`.
- `crates/smelt-runtime/src/cumulative.rs` unit — the multiset state merges by **componentwise count
  addition**; one multiset state serves multiple `π` (median, mode, exact-distinct) off the same table.
- Real-fixture e2e (examples/) — exact `MEDIAN` with a declared bounded domain has end-state equality with
  a full refresh across ≥3 partitions. Add under `crates/smelt-cli/tests/e2e/`.
- Runtime cap fail-loud — a model that **exceeds** its declared domain cap **falls back to full refresh**
  (observable: a warning + rebuild), never silent corruption or unbounded growth.
- Fail-loud (default) — an unbounded holistic aggregate **without** the opt-in refuses (default-refuse).

**Implementation shape.**
- Add a **multiset state** to C1's decomposition machinery: state table `<model>__state` stores
  `(<keys>, value, count)`; the presentation view computes the requested functional (`MEDIAN`, `MODE`, exact
  distinct) via `π`. One state, many presentations (research §14.4).
- Add the **opt-in surface** (decide its exact shape — see Open decisions; it **must be a space assertion,
  not a contract-changing strategy knob**, research §18.2). The classifier reads the opt-in; absent it, the
  holistic aggregate refuses.
- Add a **runtime cap** check on multiset cardinality with a **full-refresh fallback** when exceeded.

**Critical files.**
- `crates/smelt-logical/src/rules/cumulative.rs` — the multiset decomposition + opt-in read + default-refuse.
- `crates/smelt-runtime/src/cumulative.rs` — multiset merge, cap check, full-refresh fallback.
- `crates/smelt-core/src/config.rs` / `crates/smelt-core/src/metadata.rs` — the opt-in surface (whatever
  shape is chosen), if it is a frontmatter key.
- `crates/smelt-cli/tests/e2e/` — exact-`MEDIAN` equivalence + cap-fallback + default-refuse.
- `examples/` — a bounded-domain `MEDIAN` fixture.

**Docs touched.**
- `docs/specs/cumulative_aggregate.md` — **spec increment (pre-authorised)**: add the bounded-domain opt-in
  surface to §Surface **once its shape is chosen** (research §18.2 open question). The opt-in must be a
  **space assertion**, not a strategy knob — the SQL still just says `MEDIAN`. Add the exact holistic
  aggregators to §Surface behind the opt-in; **remove** the rung-4 Known-Divergence note.
- `docs-site/docs/guide/materializations.md` — document the opt-in and the default-refuse behaviour (timeless).

**Review checklist.**
- [ ] Exact `MEDIAN` with a declared bounded domain equals full refresh; the opt-in is a space assertion only.
- [ ] Unbounded state without opt-in refuses (fail-loud, names the approximate form / `refresh: full`).
- [ ] Exceeding the runtime cap falls back to full refresh, never silent.
- [ ] The multiset reuses C1's `(state table, view)` unit; state table not a downstream target.
- [ ] Spec/docs edits timeless; the opt-in surface added to §Surface, rung-4 Known-Divergence removed.

**Commit.** `feat(cumulative): opt-in bounded-domain multiset for exact holistic aggregates`

---

## Blocked phases

(none yet)

## Deferred during implementation

(Append-only.)

## Open decisions surfaced for the implementer

These are genuine design choices the master (research §18.2) leaves to the sub-plan. Settle each in the
owning phase; if a choice cannot be made from the spec + research, block the phase (do not guess a
contract-changing default).

- **Closed decomposition table vs registry (C2).** Settled here as a **closed, hard-coded rewrite table**
  beside `combiner_for` (`cumulative.rs:82`) — matches cumulative's "fixed allowlist, not a registry" stance
  (`cumulative_aggregate.md` §Design; research §18.2). Revisit only on a concrete custom-sketch motivator.
- **Approximate-distinct sketch representation (C2).** Which DuckDB-available state backs
  `APPROX_COUNT_DISTINCT` register-wise merge (HLL via the `hll` extension, or an approx-aggregation state
  that round-trips through the state table), and the documented error tolerance for the equivalence harness.
- **Delta-history side table: always-keep vs opt-in (C3).** Keep `<model>__deltas` for every reversible
  model (space cost, simplest), or only when reprocessing is expected. Default to always-keep for reversible
  models; note the space cost.
- **Bounded-domain opt-in surface shape (C4).** A per-model annotation vs a domain-size hint vs a runtime
  cap — it **must stay a space assertion, not a strategy knob** (research §18.2). This is the C4
  pre-authorised spec increment: choose the shape, add it to `cumulative_aggregate.md` §Surface, keep the
  SQL (`MEDIAN`) as the operator source of truth. If the shape cannot be settled, **block C4**.
- **Exact vs approximate selection (C4 vs C2).** Exact holistic aggregates go through C4's multiset;
  approximate forms (t-digest / HLL) are C2 decomposed monoids needing no opt-in. Keep the exact-vs-approximate
  choice legible to the modeller (which operator they write), not a hidden fidelity knob.

## Verification

- `cargo test` (workspace) green; `cargo clippy --all-targets` clean; `cargo fmt --all -- --check`.
- **Per-rung end-state-equivalence harness** green: `crates/smelt-cli/tests/e2e/per_partition_equivalence.rs`
  and `crates/smelt-cli/tests/cli_unit/cumulative_equivalence.rs` extended to `AVG` (C1), variance/approx-distinct
  (C2), reprocessing (C3), and exact `MEDIAN` (C4) — each equals a full refresh over the processed inputs.
- **Per-rung fail-loud test** green: `π`-purity refusal (C1), exact-distinct/`STRING_AGG` refusal (C2),
  non-group (`MIN`/`MAX`) reprocess refusal (C3), unbounded-without-opt-in refusal + cap fallback (C4).
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces`
  green — the new `examples/` cumulative fixtures build with zero diagnostics.
- `/smelt:validate cumulative_aggregate` reports zero drift for the rungs this group lands; every rung-2/3/4
  §Known-Divergence note is removed (or narrowed to the outstanding rung) as its phase lands.
