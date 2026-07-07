# Plan: The shared property-composition walk

**Date**: 2026-07-07
**Spec**: [`docs/specs/model_properties.md`](../specs/model_properties.md) §Semantics "The composition walk", §Constraints "Composition happens in the walk, not in scans"
**Spec diff**: uncommitted working tree (2026-07-07), committed with Phase 1
**Tracking PR / branch**: `worktree-incremental`
**Docs**: code-only  <!-- internal analysis architecture; spec Known-Divergences sync rides along per phase -->

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/model_properties.md` — it is the correctness oracle. Do not re-open settled spec decisions. The design basis (per-operator transfer rules, worked counterexamples) is `docs/research/20260707-property-composition-overview.md` and its nine per-property companions — consult the relevant companion before implementing a transfer function.
2. Confirm you are on branch `worktree-incremental`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — phases that change admission or scan bounds exercise the property-discovery harness (`smelt-cli::tests::property_discovery`) and/or `examples/`.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` (Salsa purity: analyses stay pure functions; fail-loud discipline).
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Spec edits describe the walk as if it has always existed; gaps land under Known Divergences in behavioural terms.
- **Property-discovery bookkeeping.** A phase that closes a catalog cell (SC-4, SC-5, SC-6, SC-7) sets that cell's `status`/`owning_test` in `docs/research/property-discovery/catalog.jsonl` (+ `catalog.md` mirror), appends the verdict to `ledger.md`, and drops a line in `unsupported.md` if a construct becomes refused.

---

## Context

`model_properties.md` §"The composition walk" requires every composition-relevant verdict to come from one shared bottom-up fold over the query tree, with per-property transfer functions; flat text scans under-derive series composition (catalog cells SC-4/SC-5) and skip CTE-internal scopes (SC-7). Today `source_bounds.rs`/`temporal.rs` are text-scan based, the batched admission gates in `rules/incremental.rs` walk only the outer UNION chain, and each proof re-implements its own traversal. This plan builds the walk and migrates the live proofs onto it; the per-operator transfer rules are already worked out in the 2026-07-07 research set.

## Scope

### In scope (spec coverage)
- §"The composition walk": the shared fold (`analysis/walk.rs`), node contexts (alias→source resolution, column lineage through projections), per-node property vector.
- §Constraints "Composition happens in the walk, not in scans": migrate partition-alignment admission, bound/reach derivation, and the monotonicity trace's CTE/set-op composition onto the walk; classify surviving scans as leaf classifiers or advisory heuristics.
- Red-first fixes for catalog cells **SC-4** (series-add reach), **SC-5** (lateness+reach sum), **SC-7** (CTE-body admission hole), **SC-6** (FD-over-UNION widening) — each closed in the phase that lands its transfer function.
- Registering the built-but-unconsumed proofs (grain/fan-out, functional dependency, discriminants, determinism predicate) as walk transfer functions so future consumers read one property vector.

### Explicitly deferred
- **SC-3** (aggregate-clock `Undecidable` widening), **SC-8** (BIT_XOR drift), **SC-9** (UDF taint fail-open) — leaf-classifier fixes independent of the walk; left to the property-discovery loop working the catalog top-to-bottom.
- **Delta-shape transfer algebra** (upsert/anti-monotone lattice, per-column mutable sets) — new capability, not a refactor; needs its own spec diff first (`20260707-property-delta-shape.md` §8).
- **Full determinism taint lattice** (membership bit, tie-ambiguity) — only the existing function-name predicate is registered as a leaf classifier here.
- **Collapsing `temporal.rs`'s advisory `EffectiveWindow` walk into the proof walk** — deliberate divergence per spec §Known Divergences; it stays an advisory heuristic.
- New consumers of the property vector (once-write transform, dimension MERGE) — tracked by the model-updates master.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     |        | 2026-07-07 |
| 2     | pending  |        |      |
| 3     | pending  |        |      |
| 4     | pending  |        |      |
| 5     | pending  |        |      |
| 6     | pending  |        |      |
| 7     | pending  |        |      |

---

### Phase 1: Walk skeleton + exhaustive scope enumeration

**Goal.** Land `analysis/walk.rs`: normalize a parsed model into a query tree (CTE definitions in dependency order → set-op branches → FROM items incl. derived tables), fold it bottom-up, and expose the first walk product — the exhaustive list of scopes (GROUP BY / DISTINCT / OVER / HAVING / set-op / LIMIT) with their nesting path, including CTE-internal ones.

**Pre-conditions.** None. Commit the pending `model_properties.md` spec diff with this phase.

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/walk.rs::tests::enumerates_scopes_inside_cte_bodies` — a model with `DISTINCT` and `HAVING` inside a CTE body yields those scopes; asserts the existing outer-chain walk (current `rules/incremental.rs` helpers) does NOT (red half documents the hole).
- `walk.rs::tests::enumerates_set_op_branch_scopes_per_branch` — UNION ALL with a `GROUP BY` in each branch yields two scopes with distinct branch paths.
- `walk.rs::tests::derived_table_and_nested_cte_scopes` — subquery-in-FROM and CTE-referencing-CTE both visited; dependency order stable.
- `walk.rs::tests::alias_resolution_through_cte_rename` — a column renamed through a CTE projection resolves to its source leaf in the node context.

**Implementation shape.** `pub struct QueryTree` built from `smelt_parser::ast::SelectStmt` (reuse `with_clause()` / `set_operation_select()` accessors); `pub trait Transfer { type Verdict; fn leaf(...); fn operator(op: &OpNode, children: &[Self::Verdict], cx: &NodeCx) -> Self::Verdict; }`; `pub fn walk<T: Transfer>(...) -> T::Verdict`; `NodeCx` carries alias→source map and projected-column lineage. Pure functions only (Salsa purity). Scope enumeration is the first `Transfer` impl (`ScopeEnum`).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/walk.rs` — new
- `crates/smelt-logical/src/analysis/mod.rs` — module registration + re-exports
- `docs/specs/model_properties.md` — commit the pending spec diff verbatim (no further edits)

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Walk visits CTE bodies, set-op branches, derived tables — no silent skip (fail-loud: an unrecognised relational construct yields an explicit `Unsupported` node, never an empty enumeration)
- [ ] Pure functions; no Salsa, no I/O
- [ ] No consumer rewiring yet (that is Phases 2+)
- [ ] Spec + docs edits are timeless

**Commit.** `feat(analysis): property-walk skeleton — query-tree fold with exhaustive scope enumeration`

---

### Phase 2: Alignment admission via the walk — close SC-7

**Goal.** The batched admission gates (HAVING / DISTINCT / OVER / LIMIT) judge every scope the walk enumerates — CTE bodies included — using the existing AST-pure `scope_{group_by,distinct,over}_alignment` as leaf classifiers; the uppercase-substring scanners are deleted or demoted to debug assertions.

**Pre-conditions.** Phase 1 walk + scope enumeration.

**TDD tests to write first.**
- `smelt-cli::tests::property_discovery::sc_7_cte_body_admission::cte_internal_cross_partition_distinct_is_refused` — linkC red: today the model is admitted and the incremental result diverges from full refresh after a late row; green = admission refuses it (this becomes SC-7's `owning_test`).
- `crates/smelt-logical/src/rules/incremental.rs::tests::cte_body_having_gated_same_as_outer` — the same HAVING construct is judged identically at top level and inside a CTE.
- Regression: existing admission tests stay green (aligned CTE-internal scopes still admitted — no blanket refusal).

**Implementation shape.** Replace `find_inadmissible_over` / the outer-chain HAVING/DISTINCT walks in `rules/incremental.rs` with a call into the walk's scope enumeration + per-scope alignment classifier; wire `scope_over_alignment` (currently test-only) as the OVER leaf classifier.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/rules/incremental.rs` — admission rewire
- `crates/smelt-logical/src/analysis/{walk.rs,mod.rs}` — alignment transfer fn
- `crates/smelt-cli/tests/property_discovery/` — SC-7 harness cell
- `docs/research/property-discovery/{catalog.jsonl,catalog.md,ledger.md,unsupported.md}` — SC-7 verdict
- `docs/specs/model_properties.md` — Known Divergences: admission-scan bullet updated (behavioural terms)

**Review checklist**:
- [ ] SC-7 harness test red-then-green; catalog/ledger updated
- [ ] No admission *widening* slipped in (this phase only closes the fail-open hole)
- [ ] Substring scanners removed from the admission path
- [ ] Spec edits timeless

**Commit.** `fix(rules): batched admission judges CTE-internal scopes via the property walk (SC-7)`

---

### Phase 3: Reach as a walk transfer function — close SC-4

**Goal.** Bound/reach derivation composes correctly: **series-add** along a nested path (stacked window frames, chained join bands), **parallel-max** across set-op branches, `NotDerivable`/`Unbounded` absorbing. `derive_and_classify_bounds` keeps its signature; internally it runs the walk instead of the flat text scan.

**Pre-conditions.** Phase 1. (Phase 2 not required, but expected done.)

**TDD tests to write first.**
- `smelt-cli::tests::property_discovery::sc_4_stacked_frames::late_row_inside_summed_reach_is_folded` — linkC red: 7d RANGE frame in a CTE under a 3d RANGE outer window; late row 8–10d back diverges today; green = scan widened to 10d (SC-4's `owning_test`).
- `crates/smelt-logical/src/analysis/walk.rs::tests::reach_series_adds_parallel_maxes` — unit: stacked frames add; UNION ALL branches max; `Symbolic` offset in series position → `NotDerivable` for that source.
- `walk.rs::tests::chained_join_bands_add_along_path` — two chained interval joins (1d + 2d) derive 3d for the far source.
- Regression: the existing per-source bound tests (`source_bounds.rs`) stay green under the walk-backed entry point.

**Implementation shape.** `ReachTransfer` impl: leaf = `parse_interval`-based frame/band/WHERE-shift extraction (the existing extraction logic, invoked per node instead of over flat text); operator rule = `then` (series add) vs `merge` (parallel max) chosen by tree position. Keep `BoundResult::merge` for parallel; add `BoundResult::then`. Delete the flat-scan path once parity tests pass.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/source_bounds.rs` — `then`, walk-backed derivation, flat scan removal
- `crates/smelt-logical/src/analysis/walk.rs` — `ReachTransfer`
- `crates/smelt-cli/tests/property_discovery/` — SC-4 cell
- `docs/research/property-discovery/*` — SC-4 verdict
- `docs/specs/model_properties.md` — Known Divergences sync

**Review checklist**:
- [ ] SC-4 red-then-green; catalog/ledger updated
- [ ] Series vs parallel chosen by tree structure, not construct name
- [ ] Fail-closed preserved: unknown operator → `NotDerivable` for sources beneath it
- [ ] No pushdown-eligibility widening beyond what correct reach implies

**Commit.** `fix(analysis): reach composes series-add/parallel-max via the property walk (SC-4)`

---

### Phase 4: Lateness + reach sum — close SC-5

**Goal.** The effective scan window widens by `source_lateness + computation_reach`, not `max` of the two.

**Pre-conditions.** Phase 3 (reach values now correct per node).

**TDD tests to write first.**
- `smelt-cli::tests::property_discovery::sc_5_lateness_plus_reach::late_row_in_gap_between_max_and_sum_is_folded` — linkC red-first (SC-5's `owning_test`). If the red test does NOT fail (an admission gate masks the site — the reach doc's open question 3), record the finding in the ledger, mark SC-5 `done (not reproducible — gated)`, and reduce this phase to the unit test below.
- `crates/smelt-logical/src/analysis/source_bounds.rs::tests::effective_window_sums_lateness_and_reach` — unit on the combination site.

**Implementation shape.** Locate the `max` combination in `compute_effective_window` (lib.rs / source_bounds.rs / temporal.rs callers) and replace with saturating sum on the scan side; write-clamp side unchanged.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/source_bounds.rs`, `crates/smelt-logical/src/lib.rs` — the combination site
- `crates/smelt-cli/tests/property_discovery/` — SC-5 cell
- `docs/research/property-discovery/*` — SC-5 verdict

**Review checklist**:
- [ ] SC-5 outcome recorded honestly (bug fixed, or gated-not-reproducible with evidence)
- [ ] Scan widened, write clamp untouched

**Commit.** `fix(analysis): effective scan window sums source-lateness and computation-reach (SC-5)`

---

### Phase 5: CTE-transparent monotonicity trace + branch-wise set-op composition

**Goal.** The event-time trace composes through the walk: a trace through a renaming CTE projection reduces to the source leaf (offsets add, strictness meets); set-op branches carry a per-branch trace vector instead of a single collapsed verdict. Injection-point classification consumes the per-branch vector (per-branch pushdown for UNION ALL stays behaviourally identical; the new capability is trace-through-CTE).

**Pre-conditions.** Phases 1, 3.

**TDD tests to write first.**
- `crates/smelt-logical/src/analysis/monotonicity.rs::tests::trace_reduces_through_renaming_cte` — red: `WITH b AS (SELECT ts AS event_ts FROM src) SELECT event_ts AS event_time FROM b` is `NotTraceable` today via name-based leaf matching; green: `Traceable{src.ts}`.
- `monotonicity.rs::tests::offsets_add_and_strictness_meets_across_layers` — CTE adds `+ INTERVAL 1 day`, outer adds `+ INTERVAL 2 hours`: folded offset; `date_trunc` in one layer weakens `is_strict`.
- `walk.rs::tests::set_op_branch_trace_vector` — UNION ALL branches anchored to different sources yield a two-entry vector, no collapsed verdict; `StaticSeed` branch refuses that branch's push.
- Equivalence guard: `smelt-cli::tests::property_discovery` happy-path cells (G-01, G-03, G-09) stay green; `cargo test -p smelt-cli --test example_diagnostics` green (the widening must not admit a wrong model in `examples/`).

**Implementation shape.** `TraceTransfer` impl: leaf = existing `trace_event_time` expression fold (unchanged, stays pure); operator rule = projection re-mapping via `NodeCx` lineage, set-op = vector, join = existing anchor resolution invoked with walk-provided alias scope. `trace_event_time_declared` widening rules unchanged.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/{monotonicity.rs,walk.rs}` — transfer fn + lineage reduction
- `crates/smelt-logical/src/rules/incremental.rs` — injection point reads the vector
- `docs/specs/model_properties.md` — Known Divergences sync (trace-through-CTE now behaviourally described)

**Review checklist**:
- [ ] Expression-level trace untouched (pure, fail-closed); only relational composition added
- [ ] Widening is exactly trace-through-projection — no new function admissions
- [ ] Nullability gate still applied after reduction (leaf nullability, post-join nullability unchanged/no regression)
- [ ] Equivalence guards green

**Commit.** `feat(analysis): monotonicity trace composes through CTEs and set-op branches via the property walk`

---

### Phase 6: Register the remaining proofs as transfer functions — close SC-6

**Goal.** Grain/fan-out, functional dependencies, discriminants, and the determinism predicate become walk-registered transfer functions producing one per-model `PropertyVector`, so every current and future consumer reads one derivation. Fixes SC-6 red-first: `FunctionalDependency.key` is read, and a declared FD over a UNION ALL body no longer widens.

**Pre-conditions.** Phases 1–5.

**TDD tests to write first.**
- `smelt-cli::tests::property_discovery::sc_6_fd_over_union::declared_fd_over_union_all_is_refused` — linkB red (SC-6's `owning_test`): same key, different determined value per branch → verdict must not be `Constant`.
- `crates/smelt-logical/src/analysis/functional_dependency.rs::tests::fd_key_field_is_consulted` — a declaration whose `key` doesn't match the join/group structure no longer widens (the parsed-but-never-read bug).
- `walk.rs::tests::group_by_establishes_grain_and_fds` — GROUP BY output carries derived key + FD-factory facts in the vector.
- `walk.rs::tests::union_all_drops_grain_and_fds_unless_discriminated` — both branches keyed → union unkeyed; literal tag column in the key → preserved (the research's discriminated-union rule).
- `walk.rs::tests::determinism_predicate_registered_as_leaf` — vector carries per-column det facts; clean ∪ clean = clean across UNION ALL.

**Implementation shape.** `GrainTransfer`, `FdTransfer` (transfer rules + Armstrong closure at each node, per `20260707-property-per-key-constancy.md` §5), `DiscriminantTransfer` (per aggregate output column), `DetTransfer` (name predicate as leaf). `pub fn model_property_vector(sql, ctx) -> PropertyVector` as the single entry. Existing pairwise entry points (`fan_out`, `functional_dependency_verdict`) delegate; signatures preserved.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/{walk.rs,join_shape.rs,functional_dependency.rs,discriminants.rs,mod.rs}`
- `crates/smelt-cli/tests/property_discovery/` — SC-6 cell
- `docs/research/property-discovery/*` — SC-6 verdict
- `docs/specs/model_properties.md` — Known Divergences sync

**Review checklist**:
- [ ] SC-6 red-then-green; declaration still widens the genuinely-undecidable single-branch case (no over-narrowing)
- [ ] Derived facts only extend fail-closed defaults (grain absent → OneToMany; FD absent → NotProven)
- [ ] No new consumers wired (transform wiring stays with the model-updates master)
- [ ] Composite-key limitation (G-10) not silently worsened; note interaction in ledger if touched

**Commit.** `feat(analysis): grain, FD, discriminant, determinism transfer functions on the property walk (SC-6)`

---

### Phase 7: Invariant + spec sync + scan classification

**Goal.** Land the architectural rule so the walk stays the single composition point: `architecture.md` gains a "Property composition walk rule" section; `CLAUDE.md` gains its invariant bullet; every surviving non-walk scan in `smelt-logical` is classified in-code (doc comment: `leaf classifier` or `advisory heuristic`) and the spec's Known Divergences reflects the end state.

**Pre-conditions.** Phases 1–6.

**TDD tests to write first.**
- `crates/smelt-logical/tests/walk_coverage.rs::admission_paths_have_no_raw_text_scans` — structural test: the admission/proof modules contain no `to_uppercase().contains(` outside functions marked as leaf classifiers/advisory (mechanism analogous to the hardening ratchet; exact mechanism implementer's choice, must fail on a new raw scan in a proof path).

**Implementation shape.** Docs + classification comments + the guard test. No behavioural change.

**Critical files (allowed to touch in this phase).**
- `docs/specs/architecture.md` — new invariant section
- `CLAUDE.md` — invariant bullet under "Architectural invariants"
- `crates/smelt-logical/src/analysis/*.rs`, `rules/*.rs` — classification doc comments only
- `crates/smelt-logical/tests/walk_coverage.rs` — guard
- `docs/specs/model_properties.md` — final Known Divergences state

**Review checklist**:
- [ ] Invariant text matches the spec's Constraints bullet (no drift between the three homes)
- [ ] Guard test fails on an injected raw scan (verified red once, then removed)
- [ ] Spec Known Divergences honest about what remains (advisory `temporal.rs` walk, deferred delta-shape/taint lattices)

**Commit.** `docs(architecture): property-composition-walk invariant + scan classification guard`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- `cargo test -p smelt-cli --tests property_discovery` — SC-4, SC-5, SC-6, SC-7 owning tests green; happy-path cells (G-01, G-03, G-09) still green.
- `cargo test -p smelt-logical` — walk unit + transfer-function tests green.
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` — examples clean.
- `cargo test -p smelt-runtime --test execute_parity` — run-pipeline parity gate green.
- `/smelt:validate model_properties` reports zero drift on §"The composition walk" and the Constraints bullet.
