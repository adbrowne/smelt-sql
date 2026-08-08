# Plan: Derived maintenance-plan proofs

**Date**: 2026-08-08
**Spec**: [`docs/specs/model_properties.md`](../specs/model_properties.md) (proof semantics: §"Footprint reflection / bounded write footprint", §"Partition-locality projection", §"Faithful-fold conditions", §"Definition-change column classification"); consumer policy stays per [`docs/specs/incremental_models.md`](../specs/incremental_models.md)
**Spec diff**: none — implements already-specified behavior; closes the Known Divergences entry "Four of the seven maintenance-plan proofs are unbuilt" in both specs
**Tracking PR / branch**: `spec-redraft-incremental-models`
**Docs**: code+docs (spec maturity rows + divergence entries only; the proofs are internal to the analysis layer — `model_properties.md` §References records "no standalone user page")

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read the spec at `docs/specs/model_properties.md` — it is the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `spec-redraft-incremental-models`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Real-fixture tests, not just AST units — every phase exercises its feature in `examples/`.
- Red-green TDD: failing test before any implementation.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call: fmt + clippy + tests + example_diagnostics, failures-only output) — do not run the four commands separately.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Don't widen scope: a phase may not reach into a later phase's scope.
- Honor architectural invariants from `CLAUDE.md` — especially the **property composition walk rule** (a composition-relevant verdict is produced by the shared walk, never an ad hoc scan; any surviving scan is a doc-tagged leaf classifier or advisory heuristic — `cargo test -p smelt-logical --test walk_coverage` scans every new file under `src/{analysis,maintenance}` automatically), **maintenance-plan purity** (proofs are pure functions in `smelt-logical`; consumers never re-derive), and **Salsa purity**.
- **Timeless-oracle rule (CLAUDE.md).** Phase vocabulary lives in *this plan file only*. Edits to the specs describe the proofs as if they have always existed; residual gaps go under **Known Divergences** in behavioural terms.

---

## Context

The maintenance plan's admission layer consumes seven proofs by name (`model_properties.md` §Interactions "The maintenance plan"); four of them — footprint reflection, partition-locality projection, faithful-fold conditions, definition-change column classification — are fully specified in §Semantics but have no classifier: the tracer hand-supplies their verdicts (`ScanClamp::footprint()` is a mirror of the read clamp; `derive::link_source` is a read-side-only heuristic; the faithful-fold conditions are inline bools; `derive_column_added` tests set-membership and group-emptiness proxies). This plan builds the four proofs as pure derivations and rewires the plan derivation onto them, without changing which shapes are admitted except where the current heuristic is provably wrong against the spec.

## Scope

### In scope (spec coverage)
- `model_properties.md` §"Faithful-fold conditions": a typed `FaithfulFold` verdict composing declared source posture with the algebraic discriminants.
- `model_properties.md` §"Footprint reflection / bounded write footprint": `reflect_footprint` with the three-way `FootprintResult` verdict, replacing the mirror in `ScanClamp::footprint()` as the derivation.
- `model_properties.md` §"Partition-locality projection": `locality_verdict` composing read bound + reflected footprint per `(cell × source)`, replacing `derive::link_source`'s margin heuristic.
- `model_properties.md` §"Definition-change column classification": `classify_definition_change` composing skeleton-role extraction, the additive-only model-diff, and per-column provenance (expression dependency resolution, not group-emptiness).
- Closing the "Four of the seven maintenance-plan proofs are unbuilt" divergence entries in both specs; fixing the stale MP14 build-order pointer.

### Explicitly deferred
- Wiring a production `ColumnAdded` trigger in `smelt-db` (`queries/maintenance.rs` derives no such trigger today; making `MaintenanceSkeletonColumnAdded` reachable needs its own diagnostics-catalogue work) — the proof lands consumer-ready; the trigger derivation is separate work, recorded in the residual divergence entry.
- Column-group-scoped dirt propagation and hour-granularity propagation (the other residues of the same divergence entry) — untouched.
- The `derive.rs` enrich-only waiver policy (`covered_by_mutation` + `source_contributes_to_fold`) stays in `derive_new_data` — it is plan policy, not part of the faithful-fold proof.
- Any change to which techniques exist or how they lower (`smelt-runtime` changes limited to consuming verdict shape changes, if any).

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 73751148 | 2026-08-08 |
| 2     | done     | b7c44bee | 2026-08-08 |
| 3     | done     | 42fbfa71 | 2026-08-08 |
| 4     | done     | dec07e11 | 2026-08-08 |
| 5     | done     | (with phase 5 commit) | 2026-08-09 |
| 6     | pending  |        |      |

---

### Phase 1: Faithful-fold conditions as a typed verdict

**Goal.** Extract the two inline admission checks in `derive_new_data` (`derive.rs:874-970`) into a pure `faithful_fold` proof returning a typed verdict, per `model_properties.md` §"Faithful-fold conditions". Admission behavior is unchanged; the verdict becomes inspectable data instead of refusal prose.

**Pre-conditions.** None (independent of the other proofs).

**TDD tests to write first.**
- `crates/smelt-logical/tests/faithful_fold.rs::monoid_over_append_only_passes_both_conditions` — `SUM` over an `AppendOnly` posture → `FaithfulFold::Holds`.
- `crates/smelt-logical/tests/faithful_fold.rs::retraction_carrying_feed_into_noninvertible_fails_condition_one_only` — retraction-carrying posture + `MIN` → fails the partition condition, passes the sub-multiset condition; the verdict names condition (1), matching the spec's "replayable feed carrying retractions into a non-invertible combiner" example with each condition independently reported.
- `crates/smelt-logical/tests/faithful_fold.rs::holistic_combiner_fails_condition_two_regardless_of_posture` — `MEDIAN` over `AppendOnly` → fails condition (2).
- `crates/smelt-logical/tests/faithful_fold.rs::verdict_reasons_preserve_admission_refusal_vocabulary` — the rendered refusal for each failing condition contains the substrings existing admission tests assert on (`"append-only"`/`"retract"`, `"independent"`, `"holistic"`/`"not a monoid"`).
- Existing green gates stay green unmodified: `maintenance_plan_admission.rs::{retractions_into_noninvertible_fail_faithful_fold, retractions_also_refuse_an_invertible_monoid, holistic_combiner_leaves_recompute_only}`, `maintenance_tracer.rs::{ex24_holistic_combiner_refuses_the_fold, ex24_mutable_source_fails_the_faithful_fold_condition}`, `maintenance_fold_contribution.rs`, `maintenance_new_data_enrich_only_waiver.rs`.
- Real fixture: `cargo test -p smelt-cli --test maintenance_conformance` unchanged (keyed pool recipes exercise the fold admission end to end).

**Implementation shape.** New `crates/smelt-logical/src/analysis/faithful_fold.rs`: `pub fn faithful_fold(combiner: SqlFunction, distinct: bool, posture: &MutationProfile, discovery: InputDeltaKind) -> FaithfulFold` with `FaithfulFold = Holds | Fails { partitioned_input: ConditionVerdict, submultiset_fold: ConditionVerdict }` (each condition independently reported, per spec "two independent conditions"). Condition (1) reads the declared posture + delta discovery; condition (2) reads `combiner_discriminants` (today's admitted set is exactly `is_monoid` — do not widen to `decomposable` in this phase). `derive_new_data` calls the proof and maps a failing verdict to the existing `Refusal::NoAdmissibleTechnique` text; the enrich-only waiver stays where it is, applied *before* the proof is consulted. Pure function, no SQL text access — nothing for walk_coverage to tag.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/faithful_fold.rs` — new proof
- `crates/smelt-logical/src/analysis/mod.rs` — module registration
- `crates/smelt-logical/src/maintenance/derive.rs` — `derive_new_data` consumes the proof
- `crates/smelt-logical/tests/faithful_fold.rs` — new tests

**Docs touched.**
- `docs/specs/model_properties.md` — §Surface "Faithful-fold conditions" maturity `not-yet` → `built`; §Known Divergences four-proofs entry shrinks to the three remaining proofs (behavioural wording).

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] The two conditions are independently sourced and independently reported (spec §"Faithful-fold conditions")
- [ ] Admitted set unchanged: no shape newly admitted or newly refused (conformance + admission tests unmodified and green)
- [ ] Waiver policy did not migrate into the proof
- [ ] No scope creep into later phases
- [ ] Spec edit is timeless and gap-first

**Commit.** `feat(logical): faithful-fold conditions as a typed derived proof`

---

### Phase 2: Footprint reflection

**Goal.** Build `reflect_footprint` per `model_properties.md` §"Footprint reflection / bounded write footprint" — the write-scope dual of `derive_model_bounds` with verdict `FootprintResult = Bounded{output_partition_col, before, after} | Unbounded | NotDerivable` — and make it the derivation behind `ScanClamp::footprint()` instead of the component swap.

**Pre-conditions.** None (Phase 1 independent).

**TDD tests to write first.**
- `crates/smelt-logical/tests/footprint_reflection.rs::symmetric_window_reach_reflects_to_the_mirror` — for a plain windowed aggregate (the shapes today's clamps admit), the derived footprint equals the current mirror `(after, before)` — the equivalence that keeps every existing clamp consumer stable.
- `crates/smelt-logical/tests/footprint_reflection.rs::trajectory_column_reflects_unbounded` — a cumulative/running-total column (value still mutable arbitrarily far downstream under late input) → `Unbounded`, the spec's canonical case; today's mirror cannot express this.
- `crates/smelt-logical/tests/footprint_reflection.rs::underivable_read_bound_reflects_not_derivable` — a source whose read bound is `NotDerivable` → footprint `NotDerivable`, never a guessed mirror.
- `crates/smelt-logical/tests/footprint_reflection.rs::bounded_verdict_names_the_output_partition_column` — the verdict carries `output_partition_col` (the output's axis), distinct from `BoundResult::Bounded`'s source column.
- Existing pinned mirror assertions (`maintenance_tracer_evolution.rs:221,448`, `smelt-runtime/tests/tracer_evolution.rs:606`) stay green — the fixtures there are symmetric-reach shapes where derivation and mirror agree; if one genuinely diverges, the assertion is updated with the derived value and the divergence explained in the commit body.
- Real fixture: `cargo test -p smelt-cli --test maintenance_conformance` (propagation/DAG cases consume `ScanClamp::footprint` via `InboundEdge::from_clamp`) unchanged.

**Implementation shape.** New `crates/smelt-logical/src/analysis/footprint.rs`: `pub fn reflect_footprint(sql: &str, ctx: &BoundContext, output_partition_col: Option<&str>) -> HashMap<String, FootprintResult>`. Reuses the walk-backed reach machinery of `source_bounds.rs` (same series/parallel composition, same interval parser) run over the write-side question; `PropertyVector::discriminants` supplies the trajectory-column `Unbounded` arm (an inverse-needing/order-monotone running fold over the output axis). Fail-closed: any construct the read-side walk refuses reflects to `NotDerivable`. `ScanClamp` gains the derived footprint as data (populated at clamp construction in `derive.rs`), with `footprint()` reading it; a `Bounded` derived verdict equal to the mirror is the expected common case. Where the derivation yields `Unbounded`/`NotDerivable` for a shape that currently receives a clamp, that clamp's cell falls back per existing fail-closed rules — expected to be unreachable for currently-admitted shapes (the admission gates already exclude trajectory writes from clamped techniques); a conformance failure here is a real finding, stop and surface it.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/footprint.rs` — new proof
- `crates/smelt-logical/src/analysis/mod.rs` — module registration
- `crates/smelt-logical/src/maintenance/mod.rs` — `ScanClamp` carries the derived footprint
- `crates/smelt-logical/src/maintenance/derive.rs` — clamp construction populates it
- `crates/smelt-logical/tests/footprint_reflection.rs` — new tests
- `crates/smelt-logical/tests/maintenance_tracer_evolution.rs`, `crates/smelt-runtime/tests/tracer_evolution.rs` — only if a pinned mirror assertion genuinely diverges

**Docs touched.**
- `docs/specs/model_properties.md` — §Surface "Footprint reflection" maturity → `built`; divergence entry shrinks accordingly.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Verdict shape matches spec exactly (`Bounded{output_partition_col, before, after} | Unbounded | NotDerivable`)
- [ ] Walk rule honored: reach composition via the shared walk machinery, no new ad hoc text scans (walk_coverage green)
- [ ] Propagation (`InboundEdge::from_clamp`) behavior unchanged for all conformance DAG cases
- [ ] No scope creep into Phase 3 (no locality changes)
- [ ] Spec edit is timeless and gap-first

**Commit.** `feat(logical): derive footprint reflection as the write-scope dual of the read bound`

---

### Phase 3: Partition-locality projection

**Goal.** Build `locality_verdict` per `model_properties.md` §"Partition-locality projection" — `Local | NotLocal{reason}` per `(cell × source)`, composing the read-side bound and the Phase-2 reflected footprint — and replace `derive::link_source`'s margin heuristic (`before > 0 || after > 0` as cross-axis evidence) at its four call sites.

**Pre-conditions.** Phase 2 done (`reflect_footprint` exists).

**TDD tests to write first.**
- `crates/smelt-logical/tests/locality_projection.rs::same_axis_bounded_read_and_footprint_is_local` — bounded read + bounded reflected footprint on the shared axis → `Local`.
- `crates/smelt-logical/tests/locality_projection.rs::cross_axis_source_with_explicit_interval_predicate_is_local` — a source whose partition column differs from the output's, linked by an explicit derivable interval predicate → `Local` (the predicate, not the margin, is the evidence).
- `crates/smelt-logical/tests/locality_projection.rs::cross_axis_source_without_predicate_is_not_local` — no derivable predicate relating the two columns → `NotLocal{reason}` naming the missing link, even if a nonzero margin exists elsewhere in the query — the false positive the heuristic admits today.
- `crates/smelt-logical/tests/locality_projection.rs::unbounded_footprint_is_not_local_even_with_bounded_read` — Phase 2's `Unbounded` reflection defeats locality regardless of the read bound (the composition the spec requires and `link_source` cannot express).
- `crates/smelt-logical/tests/locality_projection.rs::verdicts_are_per_cell_per_source` — a cell with one local and one non-local source carries both verdicts; the cell-level `PartitionLocal` folds them (first failure wins, as today).
- Existing green gates stay green: `maintenance_tracer.rs` / `maintenance_tracer_evolution.rs` locality assertions, `maintenance_plan_refusals.rs`, `maintenance_plan_conformance.rs` EX-08, `coverage_matrix_gaps.rs` `ScanUnbounded` rows, `smelt-db` `maintenance_diagnostics.rs` (`MaintenanceScanUnbounded` + `allow_full_scan` clearing), `explain_model.rs` renderings, and the full `maintenance_conformance` pool.
- Real fixture: `examples/timeseries` + `examples/web_analytics` diagnostics unchanged (`cargo test -p smelt-cli --test example_diagnostics`).

**Implementation shape.** New `crates/smelt-logical/src/analysis/locality_projection.rs`: `pub fn locality_verdict(read: &BoundResult, footprint: &FootprintResult, source_axis: Option<&str>, output_axis: Option<&str>, cross_axis_link: CrossAxisLink) -> LocalityVerdict` — a pure composition; the cross-axis evidence (`CrossAxisLink = SameAxis | ExplicitPredicate | None`) is derived where the bound derivation already discovers the linking predicate (interval join bands / `WHERE` shifts name both columns), threaded out of `source_bounds.rs` rather than re-scanned. `derive.rs` replaces the `link_source` match at its four call sites (`derive_mutation`, `append_model_edge_cells`, `read_locality`, `derive_column_added`'s loop) with the proof, keeping each site's *policy* (creation cells never refuse; K8 refusal honors `allow_full_scan`). The keyed-grain sentinel `PartitionLocal::Yes` for axis-free outputs stays, re-documented as vacuous-locality policy in `derive.rs`, not a proof verdict. `PartitionLocal` stays two-armed per spec. Behavioral deltas are confined to the two heuristic misclassifications (cross-axis-no-predicate-but-nonzero-margin → now `NotLocal`; the reviewer verifies via conformance that no *currently-generated* recipe regresses — if one does, it is a real admission bug being fixed, document it in the commit body).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/locality_projection.rs` — new proof
- `crates/smelt-logical/src/analysis/source_bounds.rs` — expose the cross-axis link evidence alongside the bound
- `crates/smelt-logical/src/analysis/mod.rs` — module registration
- `crates/smelt-logical/src/maintenance/derive.rs` — four call sites consume the proof; `link_source` deleted
- `crates/smelt-logical/src/maintenance/mod.rs` — only if the per-source verdict record needs a carrier on the cell
- `crates/smelt-logical/tests/locality_projection.rs` — new tests

**Docs touched.**
- `docs/specs/model_properties.md` — §Surface "Partition-locality projection" maturity → `built`; divergence entry shrinks.
- `docs/specs/incremental_models.md` — if the per-source verdict becomes visible in `smelt explain`, the illustrative plan rendering is refreshed (timeless).

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] Cross-axis rule matches spec: explicit derivable predicate or nothing — no margin inference ("never an inferred guess")
- [ ] Read bound AND reflected footprint both enter the verdict (spec composition), not read-side only
- [ ] Policy stayed at call sites (creation-cell no-refuse, `allow_full_scan`, keyed vacuous-locality) — the proof is policy-free
- [ ] `PartitionLocal` consumers in `smelt-runtime` (`execute.rs`, `maintenance_driver.rs`, `propagation.rs`) unchanged or updated for data-shape only, no behavior fork changes
- [ ] Spec edits are timeless and gap-first

**Commit.** `feat(logical): derive partition locality from read bound + reflected footprint per (cell, source)`

---

### Phase 4: Definition-change column classification

**Goal.** Build `classify_definition_change` per `model_properties.md` §"Definition-change column classification" — `SkeletonAdd{reason} | PureBackfill | UpstreamRederive` composing skeleton-role extraction, the additive-only model-diff, and per-column provenance — and rewire `derive_column_added` onto it, replacing the `skeleton_columns.contains()` and `mutation_sensitivity.is_empty()` proxies.

**Pre-conditions.** None of Phases 1–3 strictly required; runs after them per plan order.

**TDD tests to write first.**
- `crates/smelt-logical/tests/definition_change.rs::grouping_key_add_is_skeleton_add_with_role` — an added column in `GROUP BY` position → `SkeletonAdd` whose reason names the `SkeletonRole` (`Grouping`), which today's flattened-set test loses.
- `crates/smelt-logical/tests/definition_change.rs::pure_function_of_stored_columns_is_pure_backfill` — added column derivable from existing target columns (dependency resolution over the expression, per the additive-only diff's `collect_dependencies`) → `PureBackfill`.
- `crates/smelt-logical/tests/definition_change.rs::append_only_source_read_is_upstream_rederive_not_pure_backfill` — an added column reading an append-only source non-aggregated → `UpstreamRederive`; the group-emptiness proxy misclassifies this `PureBackfill` today (empty mutation-sensitivity ≠ no upstream read) — this is the corrected misclassification this phase exists for.
- `crates/smelt-logical/tests/definition_change.rs::non_additive_diff_refuses_classification` — a change that is not a pure addition → fail-closed refusal, never a guessed verdict.
- `crates/smelt-logical/tests/definition_change.rs::unclassifiable_shape_fails_closed` — a shape `skeleton_roles` returns `None` for → fail-closed refusal.
- Existing green gates stay green: `maintenance_tracer.rs::ex36_pure_function_field_add_is_in_place_update_with_ledger_catch_up`, the EX-39 `SkeletonColumnAdded` assertions (`maintenance_tracer.rs:550`, `maintenance_tracer_evolution.rs:468`), `maintenance_coverage_matrix.rs`, `gate.rs::column_add_between_runs_recovers_equivalence` (conformance: schema-evolution recipes).
- Real fixture: `cargo test -p smelt-cli --test maintenance_conformance` (the `AddPayloadColumn`/`AddGroupingColumn` recipes) unchanged in outcome.

**Implementation shape.** New `crates/smelt-logical/src/analysis/definition_change.rs`: `pub fn classify_definition_change(added_column: &ColumnDef, sql: &str, ctx: &DefinitionChangeCtx) -> Result<DefinitionChangeClass, ClassifyRefusal>` where the three legs call the existing owners: `maintenance::skeleton::skeleton_roles` (leg 1 — refuse `SkeletonAdd` with the role), `analysis::model_diff::additive_only_diff` (leg 2 — the proof derives it from old/new column sets instead of trusting a caller-supplied `Option<&ModelDiff>`), and expression dependency resolution via `model_diff::collect_dependencies` against the stored-column set (leg 3 — `PureBackfill` iff every dependency is an already-stored column; any source-reaching reference → `UpstreamRederive`), mirroring the check backbuild's added-column classification already performs. `derive_column_added` consumes the verdict; `ModelInputs::column_add_proof` is replaced by the internally-derived diff (callers pass old columns instead of a pre-made proof — `smelt-db`'s production `None` becomes "no old schema ⇒ fail-closed", same behavior as today). Production `ColumnAdded` trigger derivation stays out of scope. `SkeletonAdd` reasons carry the role; the `Refusal::SkeletonColumnAdded` surface is unchanged.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/analysis/definition_change.rs` — new proof
- `crates/smelt-logical/src/analysis/mod.rs` — module registration
- `crates/smelt-logical/src/maintenance/derive.rs` — `derive_column_added` consumes the verdict; `ModelInputs::column_add_proof` retired
- `crates/smelt-db/src/queries/maintenance.rs` — caller signature only (passes old-column data / stays fail-closed); no new trigger derivation
- `crates/smelt-logical/tests/definition_change.rs` — new tests

**Docs touched.**
- `docs/specs/model_properties.md` — §Surface "Definition-change column classification" maturity → `built`; the stale MP14 build-order pointer in the divergence entry corrected as the entry shrinks.

**Review checklist** (material findings only):
- [ ] TDD tests listed above exist and assert what's specified
- [ ] The `PureBackfill` leg is dependency-resolution-based, not sensitivity-emptiness; the append-only-source misclassification is fixed and pinned by a test
- [ ] Composes the three named owner proofs — no private re-implementation of skeleton roles or the diff
- [ ] Fail-closed on unclassifiable shapes and non-additive diffs
- [ ] No production `ColumnAdded` trigger wiring (deferred; `MaintenanceSkeletonColumnAdded` reachability unchanged)
- [ ] Spec edit is timeless and gap-first

**Commit.** `feat(logical): definition-change column classification as a composed derived proof`

---

### Phase 5: Divergence closure and reference sweep

**Goal.** Close the "Four of the seven maintenance-plan proofs are unbuilt" entries in both specs, leaving the genuine residues as their own behavioural gap entries; verify zero drift.

**Pre-conditions.** Phases 1–4 done.

**TDD tests to write first.** (Docs phase — the "tests" are the standing gates:)
- `bash .claude/scripts/verify-phase.sh` green.
- `cargo test -p smelt-logical --test walk_coverage` green (all four new modules classified or scan-free).
- `/smelt:validate model_properties` reports no drift on the four proof rows.

**Implementation shape.** In `model_properties.md`: delete the four-proofs divergence entry (all four now `built` in §Surface); promote any settled residue notes. In `incremental_models.md` §Known Divergences: rewrite the corresponding entry to its surviving residues only — column-group dirt coarsens to whole-partition; hour granularity declared but day-ordinal propagation; no production `ColumnAdded` trigger (so `MaintenanceSkeletonColumnAdded` remains unmapped in `smelt-db`) — each gap-first with a tracking pointer. Update `model_properties.md` §References → Code with the four new module paths; correct §References → Plans (MP5/MP6/MP14 note superseded by this plan).

**Critical files (allowed to touch in this phase).**
- `docs/specs/model_properties.md`
- `docs/specs/incremental_models.md`

**Docs touched.** (This phase *is* the docs work.)

**Review checklist** (material findings only):
- [ ] No fully-landed entry survives as a divergence; residues are gap-first with tracking pointers
- [ ] Every `§"…"` reference introduced resolves to a real heading
- [ ] Timeless: `rg -n 'Phase [A-Z0-9]|ratified|Historical name' docs/specs/model_properties.md docs/specs/incremental_models.md` clean in body sections
- [ ] `/smelt:validate` drift report triaged

**Commit.** `docs(spec): four maintenance-plan proofs derived — divergence entries closed`

---

### Phase 6: Mutation campaign over the new proof layer

**Goal.** Run a `cargo-mutants` campaign over the four new proof modules and the rewired `derive.rs` regions, kill or triage every surviving mutant, and record the residue — the same gate-attribution pattern as the prior campaign (`docs/plans/` mutation-campaign history; staged `--iterate` runs).

**Pre-conditions.** Phases 1–5 done.

**TDD tests to write first.** The campaign *produces* the test list: each surviving mutant that represents a real coverage gap gets an explicit killing test added to the relevant proof test file before the survivor is re-run; each survivor that is provably equivalent or unreachable is recorded with its justification.

**Implementation shape.** `cargo mutants --package smelt-logical --file 'src/analysis/faithful_fold.rs' --file 'src/analysis/footprint.rs' --file 'src/analysis/locality_projection.rs' --file 'src/analysis/definition_change.rs' --file 'src/maintenance/derive.rs' --iterate`, tier-1 gates first (`-p smelt-logical` unit/tracer/admission suites) then the conformance gate for stubborn survivors. Kill-tests land in the phase's proof test files; a survivors ledger (mutant → verdict → gate or justification) is appended to this plan under "Deferred during implementation" if any residue remains.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/tests/{faithful_fold,footprint_reflection,locality_projection,definition_change}.rs` — kill tests
- This plan file — survivors ledger

**Docs touched.** None (test-only phase).

**Review checklist** (material findings only):
- [ ] Every surviving mutant is either killed by a new test or ledgered with an equivalence/unreachability justification
- [ ] Kill tests assert proof semantics (spec vocabulary), not implementation internals
- [ ] No production code changes smuggled in (unless a mutant exposes a real bug — then it gets its own red-green fix and a note here)

**Commit.** `test(logical): mutation campaign over the derived proof layer`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

How to confirm the spec is satisfied at the end:
- `cargo test -p smelt-logical` — the four new proof test files plus all existing tracer/admission/refusal suites
- `cargo test -p smelt-cli --test maintenance_conformance` — the generative equivalence gate over the full recipe pool
- `cargo test -p smelt-logical --test walk_coverage` — every new module classified under the walk rule
- `cargo test -p smelt-runtime --test statement_parity` — statement emission unchanged
- `bash .claude/scripts/verify-phase.sh`
- `/smelt:validate model_properties` reports zero drift on the four proof rows
