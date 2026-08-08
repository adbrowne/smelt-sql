# Plan: Sensitivity precision — walk-composed grouping and reachable targeted techniques

**Date**: 2026-08-09
**Spec**: [`docs/specs/model_properties.md`](../specs/model_properties.md) §"Per-column mutation-sensitivity / column provenance" (including the closure-pruning rule), §"Skeleton-source closure"; [`docs/specs/incremental_models.md`](../specs/incremental_models.md) §"The plan matrix" (sensitivity kinds and the closure pruning), §"Per-cell admission"
**Spec diff**: the closure-pruning rule added alongside this plan (membership sensitivity pruned by a `Closed` skeleton-source closure over a provably outer join; declared-`referential_integrity` route excluded until its tripwire exists) — committed with this plan file
**Tracking PR / branch**: `spec-redraft-incremental-models`
**Docs**: code+docs

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session, phase by phase with implementer + reviewer subagents.

1. Read this entire plan, then the two spec sections named above — they are the correctness oracle.
2. Confirm branch `spec-redraft-incremental-models`.
3. Execute the next `pending` phase: implementer subagent (red-green TDD on the listed tests) → reviewer subagent (material findings only, spec as oracle) → fix → commit with the phase's `Commit.` line → push → record in Progress tracking.
4. Verification gate per phase: `bash .claude/scripts/verify-phase.sh` plus `cargo test -p smelt-cli --test maintenance_conformance` and `cargo test -p smelt-logical --test walk_coverage`.
5. Pause for the user if: a reviewer repeats the same material finding twice; a test cannot go green without violating a spec rule; a pre-existing unrelated failure appears.
6. Timeless-oracle rule: phase vocabulary only in this file; spec/docs edits describe behavior as always-existing; residues go to Known Divergences gap-first.

## Context

`maintenance/grouping.rs` derives value and membership sensitivity over a single top-level `SELECT` and collapses the whole model to one degenerate group on any CTE, set operation, derived table, or unresolvable reference. Separately, every ordinary equi-join enrichment attaches membership sensitivity through its `ON` read, so `Technique::ColumnScopedMerge` is unreachable from real SQL and `Technique::InPlaceUpdate` has no production consumer. The walk (`analysis/walk.rs` `ColumnLineage`/`resolve_leaf`) and the fingerprint projection (`analysis/fingerprint.rs`) already demonstrate the composed-provenance pattern; the skeleton-source closure proof exists and is consumed by recompute-restriction. This plan composes grouping through the walk, prunes membership sensitivity with the closure proof per the spec rule landed alongside it, and lowers both targeted techniques end to end under the conformance oracle.

## Scope

### In scope
- Walk-composed value sensitivity (CTE/derived-table transparency via `ColumnLineage`; blanket CTE/set-op collapse replaced by `has_unsupported` fail-closure).
- Walk-composed membership sensitivity (per-scope admission reads, resolved through the walk).
- Closure-pruned membership sensitivity (spec rule above; provably-outer-join route only).
- `ColumnScopedMerge` reachable and conformance-proven from a real enrichment shape.
- `InPlaceUpdate` lowering: production `ColumnAdded` trigger derivation from the deployed-schema snapshot, runtime statement arm + dispatch, conformance recipe; `MaintenanceSkeletonColumnAdded` becomes reachable (diagnostics catalogue entry).
- Registry hygiene: stale `known_bug_technique_pin_inert` entry; stale `MutableEnrichedRecipe` doc comment.

### Explicitly deferred
- Loosening the ON-read-is-membership rule by any means other than the closure proof (no key-pinning heuristics).
- Aggregating-scope enrichment closures (the closure proof's v1 restriction stands).
- Change-feed membership consumption (`AppendOnly`-source membership remains not contributed, per the membership-sensitivity plan's deferral).
- The declared-`referential_integrity` closure route for membership pruning (excluded until the count-preservation tripwire exists).

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | b8f124cc | 2026-08-09 |
| 2     | done     | 6a51ebaf | 2026-08-09 |
| 3     | done     | c7703b36 | 2026-08-09 |
| 4     | done     | 93fc0622 | 2026-08-09 |
| 5     | pending  |        |      |
| 6     | pending  |        |      |
| 7     | pending  |        |      |

---

### Phase 1: Registry and comment hygiene

**Goal.** Close the stale `known_bug_technique_pin_inert` divergence-registry entry (the pin ladder is live via `effective_override` → `resolve_cell_choice`; the entry's grep target no longer exists) and fix `MutableEnrichedRecipe`'s doc comment claiming `ColumnScopedMerge` eligibility it no longer has.

**TDD tests to write first.**
- The registry's own staleness check is the test: `cargo test -p smelt-cli --test maintenance_conformance` with the entry removed/reclassified must pass, and `divergence_registry_staleness_report` must no longer flag it.

**Implementation shape.** Remove (or reclassify as resolved) the entry in `crates/smelt-cli/tests/maintenance_conformance/registry.rs:100-106` and its `known_bug_still_reproduces` grep arm; correct the doc comment at `crates/smelt-maintenance-testkit/src/recipe.rs:629` to describe the membership-recompute outcome.

**Critical files.** `crates/smelt-cli/tests/maintenance_conformance/registry.rs`, `crates/smelt-maintenance-testkit/src/recipe.rs`.

**Docs touched.** None (test/registry hygiene).

**Review checklist.**
- [ ] The pin-ladder liveness claim verified against `maintenance_driver.rs` (not just the grep)
- [ ] No behavior change

**Commit.** `test(conformance): close stale technique-pin-inert registry entry; fix enriched-recipe doc`

---

### Phase 2: Walk-composed value sensitivity

**Goal.** Rebuild `derive_column_groups`' value-sensitivity pass on the walk's `ColumnLineage`, so a simple rename chain through arbitrarily nested single-use CTEs/derived tables resolves to its base relation instead of collapsing, per `model_properties.md` §"Per-column mutation-sensitivity" and §"The composition walk". The blanket `with_clause().is_some() || has_set_operation()` collapse is replaced by walk-level fail-closure (`has_unsupported`, top-level set-op refusal — same posture as `fingerprint_projection`).

**Pre-conditions.** Phase 1 done.

**TDD tests to write first.**
- `crates/smelt-logical/tests/maintenance_grouping.rs::cte_wrapped_enrichment_derives_nondegenerate_groups` — the same enrichment join that groups cleanly at top level, wrapped in a single-use CTE, produces the same groups (no degenerate collapse).
- `crates/smelt-logical/tests/maintenance_grouping.rs::derived_table_rename_chain_resolves_to_base_relation` — a column renamed through a derived table carries its base relation's sensitivity.
- `crates/smelt-logical/tests/maintenance_grouping.rs::aggregate_over_cte_renamed_column_finds_base_source` — `SUM(x)` where `x` is a CTE rename of a mutable source's column attributes value sensitivity to that source (the lineage-into-aggregate-argument case).
- `crates/smelt-logical/tests/maintenance_grouping.rs::unsupported_tree_still_collapses` — a shape the walk cannot normalize (and a top-level set operation) still degenerates whole-model, reasons preserved.
- `crates/smelt-logical/tests/maintenance_grouping.rs::ambiguous_self_join_still_collapses` — `ColumnLineage.ambiguous` fails closed.
- Existing `maintenance_grouping.rs` suite green unmodified; `smelt explain` degenerate report (`explain_model.rs`) unchanged for the still-degenerate shapes.

**Implementation shape.** In `grouping.rs`: build `QueryTree::from_sql`; per output column use its `ColumnLineage.leaf` (simple rename chain) for source attribution; for computed/aggregate expressions run the existing per-item classifier as a leaf classifier over that scope's own items, resolving embedded refs via the scope's alias map plus one lineage lookup for CTE/derived-table aliases (the `resolve_reference_leaf` mechanism, generalized off the top scope). The aggregate-gates rule (AppendOnly contributes only under aggregation) is preserved per attribution. Membership pass unchanged in this phase (still outer-scope; still collapses on shapes it can't see — Phase 3's job). Module doc rewritten: the single-scope restriction paragraph becomes the walk-composed description with the surviving leaf classifiers tagged.

**Critical files.** `crates/smelt-logical/src/maintenance/grouping.rs`, `crates/smelt-logical/src/analysis/walk.rs` (only if `resolve_reference_leaf` needs a scope-generalized variant), `crates/smelt-logical/tests/maintenance_grouping.rs`.

**Docs touched.**
- `docs/specs/model_properties.md` — §Surface mutation-sensitivity row maturity: single-scope restriction wording updated (value pass walk-composed; membership pass still single-scope — honest split).

**Review checklist.**
- [ ] Fail-closed preserved: every previously-collapsing shape either resolves correctly or still collapses — no optimistic attribution
- [ ] Walk rule: no new raw text scans; leaf classifiers doc-tagged; walk_coverage green
- [ ] AppendOnly-under-aggregation rule preserved through lineage attribution
- [ ] Conformance pool green (no generated recipe changes outcome)

**Commit.** `feat(logical): walk-composed value sensitivity — column groups survive CTEs and derived tables`

---

### Phase 3: Walk-composed membership sensitivity

**Goal.** Extend the membership pass to every scope the walk enumerates: each scope's JOIN-`ON`/`WHERE`/`HAVING` conjuncts are scanned with refs resolved through the walk, so a mutable dimension joined inside a CTE is seen; the union still attaches uniformly to the scope's output rows and composes up the tree. Fail-closure unchanged (USING/no-ON, subquery conjuncts, unresolvable refs).

**Pre-conditions.** Phase 2 done.

**TDD tests to write first.**
- `crates/smelt-logical/tests/maintenance_grouping.rs::cte_interior_mutable_join_attaches_membership` — mutable dim joined inside a CTE attaches membership sensitivity at the model level.
- `crates/smelt-logical/tests/maintenance_grouping.rs::membership_composes_across_nested_scopes` — two scopes each contributing a distinct mutable admission source union at the top.
- `crates/smelt-logical/tests/maintenance_grouping.rs::subquery_conjunct_still_fails_closed` — unchanged posture inside a CTE scope too.
- Existing membership tests (`gate.rs::keyed_enriched_recipe_admits_membership_recompute`, change-feed recompute tests) green unmodified.

**Implementation shape.** The membership scan becomes a per-scope walk step (or post-walk per-scope loop over the tree's normalized scopes) reusing the existing conjunct splitter; refs resolve via each scope's alias map; a scope the walk can't normalize collapses as today. No pruning yet.

**Critical files.** `crates/smelt-logical/src/maintenance/grouping.rs`, `crates/smelt-logical/tests/maintenance_grouping.rs`.

**Docs touched.**
- `docs/specs/model_properties.md` — mutation-sensitivity maturity note: both passes walk-composed.

**Review checklist.**
- [ ] Membership never lost by composition (a contributing inner scope always reaches the top)
- [ ] Fail-closed unchanged for the enumerated constructs
- [ ] Conformance green

**Commit.** `feat(logical): walk-composed membership sensitivity across nested scopes`

---

### Phase 4: Closure-pruned membership sensitivity

**Goal.** Implement the spec's pruning rule: an enrichment join whose skeleton-source closure is `Closed` with row preservation from a provably outer join contributes no membership sensitivity through its own `ON` equality. Declared-`referential_integrity` closures do not prune. Everything else unchanged.

**Pre-conditions.** Phases 2–3 done.

**TDD tests to write first.**
- `crates/smelt-logical/tests/maintenance_grouping.rs::closed_outer_enrichment_join_prunes_membership` — LEFT JOIN dim with `Closed` closure → dim absent from membership sets; present in value sensitivity for its select-item columns.
- `crates/smelt-logical/tests/maintenance_grouping.rs::declared_ri_inner_join_does_not_prune` — inner join closed only via declared `referential_integrity` → membership stays.
- `crates/smelt-logical/tests/maintenance_grouping.rs::open_closure_does_not_prune` — any failing conjunct (membership predicate on enrichment column, fan-out, skeleton column from enrichment side) → membership stays.
- `crates/smelt-logical/tests/maintenance_tracer.rs` (or a new tracer case) — the pruned shape's `UpstreamMutation(dim)` cell now derives `Corner::ColumnMerge`/`Technique::ColumnScopedMerge`.

**Implementation shape.** The membership pass consults `skeleton_source_closure` for each enrichment join before attaching that join's `ON` contribution; the closure verdict must carry (or be paired with) whether row preservation came from join shape vs declaration — extend the closure's verdict data if needed (additive field, no behavior change for existing consumers). Pruning applies only to the join's own equality read; the same source read anywhere else still contributes.

**Critical files.** `crates/smelt-logical/src/maintenance/grouping.rs`, `crates/smelt-logical/src/analysis/skeleton_closure.rs` (verdict provenance field only), `crates/smelt-logical/tests/maintenance_grouping.rs`, `crates/smelt-logical/tests/maintenance_tracer.rs`.

**Docs touched.**
- `docs/specs/model_properties.md` / `docs/specs/incremental_models.md` — Known Divergences: the "ColumnScopedMerge unreachable" entry shrinks to execution-pending (Phase 5 closes it).

**Review checklist.**
- [ ] Pruning is proof-gated and join-shape-restricted exactly per spec; RI route refused
- [ ] Only the pruned join's own ON equality is exempted — other reads of the same source still attach
- [ ] Existing membership-recompute conformance cases unchanged (their recipes use inner joins)

**Commit.** `feat(logical): closure-proven outer enrichment joins prune membership sensitivity`

---

### Phase 5: ColumnScopedMerge end-to-end under the conformance oracle

**Goal.** A generated recipe with a LEFT-JOIN closed enrichment (dimension value mutated between runs) derives a live `ColumnScopedMerge` cell, executes it through `execute_project`, and matches the full-refresh oracle — including the departed-dimension-row case (values re-derive to NULL without membership change).

**Pre-conditions.** Phase 4 done.

**TDD tests to write first.**
- `crates/smelt-maintenance-testkit/src/recipe.rs`: new `ValueEnrichedRecipe` (LEFT JOIN, closure-satisfying shape) staged alongside `MutableEnrichedRecipe`.
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::value_enriched_recipe_executes_column_scoped_merge` — asserts the derived plan carries `Technique::ColumnScopedMerge`, the executed statement family is the column-scoped MERGE (statement-parity seam), and end-state equals the oracle across dimension value mutation, dimension row deletion, and re-run schedules.
- `crates/smelt-runtime --test statement_parity` extended with the now-reachable family leg if the harness requires it.

**Implementation shape.** Mostly test/testkit work; any runtime gap the recipe exposes (e.g. `resolve_live_column_scoped_cell` conditions) is fixed red-green here. The known-bug ledger entry for schema-snapshot skipping is untouched.

**Critical files.** `crates/smelt-maintenance-testkit/src/recipe.rs`, `crates/smelt-cli/tests/maintenance_conformance/{gate.rs,mod.rs}`, `crates/smelt-runtime/src/{execute.rs,maintenance_driver.rs}` (only if the recipe exposes a dispatch gap).

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: "`ColumnScopedMerge` is currently unreachable from any shipped SQL shape" entry deleted (landed); `docs-site` page for incremental models updated if it names the limitation.

**Review checklist.**
- [ ] Equivalence asserted through the real pipeline against a real backend, not unit-mocked
- [ ] Departed-row case covered
- [ ] Divergence entry deletion justified by the passing gate

**Commit.** `test(conformance): closure-pruned enrichment executes ColumnScopedMerge and matches the oracle`

---

### Phase 6: InPlaceUpdate lowering with a production ColumnAdded trigger

**Goal.** Derive the `ColumnAdded` trigger in production from the deployed-schema snapshot (old columns), lower `Technique::InPlaceUpdate` in `smelt-runtime` (statement arm via `emit_in_place_update` + driver dispatch), make `MaintenanceSkeletonColumnAdded` reachable (diagnostics catalogue entry), and prove equivalence for a column-add schedule through the conformance gate.

**Pre-conditions.** Phases 2–4 done (Phase 5 independent). Definition-change proof (20260808 plan) landed.

**TDD tests to write first.**
- `crates/smelt-db/tests/maintenance_diagnostics.rs::column_added_trigger_derived_from_deployed_schema` — with a stored schema snapshot differing by an added derivable column, the plan carries the `ColumnAdded` trigger and the `PureBackfill` cell; a skeleton add surfaces `MaintenanceSkeletonColumnAdded`.
- `crates/smelt-db/tests/diagnostics_catalogue.rs` — new code entry.
- `crates/smelt-runtime/tests/technique_lowering.rs::in_place_update_lowered_from_pure_backfill_cell` — statement built via the pure emitter (statement-parity leg; the diagnostics.rs:574 refusal deleted).
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::pure_backfill_column_add_executes_in_place_update` — schedule: run, edit model adding a stored-column-derived column, run; assert `InPlaceUpdate` executed and end state equals the oracle. Also fixes/closes the `known_bug_incremental_path_skips_schema_snapshot` ledger entry (snapshot must be saved on the incremental path for the trigger to see it — red-green that first).
- The `ex39b`/EX-39 refusal directions re-asserted end-to-end where the trigger is real.

**Implementation shape.** `smelt-db/src/queries/maintenance.rs`: read the deployed-schema snapshot (the store `schema_evolution` already consults), diff against current projection, populate `old_columns` and push the `ColumnAdded` trigger (thin Salsa wrapper; the diff/classification stays in `smelt-logical`). `smelt-runtime`: `build_technique_statements` arm calling `emit_in_place_update` with the added columns' resolved assignments and region predicate; `maintenance_driver` dispatch arm parallel to the column-scoped path; `save_deployed_schema` called on the incremental path too. Map `Refusal::SkeletonColumnAdded` in `map_metadata_error_to_diagnostic`'s exhaustive match (compile-gated).

**Critical files.** `crates/smelt-db/src/queries/maintenance.rs`, `crates/smelt-db/src/lib.rs`, `crates/smelt-runtime/src/{diagnostics.rs,maintenance_driver.rs,execute.rs}`, `crates/smelt-state` (only if snapshot read needs exposing), testkit + conformance files above.

**Docs touched.**
- `docs/specs/incremental_models.md` — Known Divergences: emission-remainder (`emit_in_place_update` no consumer) and proof-layer residue (no production `ColumnAdded` trigger) entries deleted; §Surface diagnostics table row for `MaintenanceSkeletonColumnAdded` marked reachable if the table tracks that.
- `docs-site/docs/` — the schema-evolution/incremental page gains the column-add repair description (timeless).

**Review checklist.**
- [ ] Statement authored by the pure emitter only (statement-parity + no-authoring gates green)
- [ ] Salsa purity: the query assembles inputs; classification stays pure in smelt-logical
- [ ] `MetadataError`/diagnostic exhaustiveness gates compile
- [ ] Snapshot-skipping known-bug closed with evidence, not just entry removal
- [ ] Conformance equivalence across the edit schedule

**Commit.** `feat(runtime): lower InPlaceUpdate from a production ColumnAdded trigger, oracle-proven`

---

### Phase 7: Divergence closure and doc sync

**Goal.** Sweep both specs' Known Divergences for entries this plan landed (grouping fail-closed collapse scope, ColumnScopedMerge unreachability, InPlaceUpdate remainder, plan-consumer gaps that referenced them); update `docs-site` incremental pages; run `/smelt:validate` for both specs and triage.

**TDD tests to write first.** Standing gates only: `verify-phase.sh` full, `maintenance_conformance`, `walk_coverage`, `statement_parity`, `/smelt:validate model_properties` + `/smelt:validate incremental_models` drift triage.

**Critical files.** `docs/specs/model_properties.md`, `docs/specs/incremental_models.md`, `docs-site/docs/` incremental pages.

**Review checklist.**
- [ ] No fully-landed entry survives; residues gap-first with tracking pointers
- [ ] Timeless lint clean
- [ ] Drift reports triaged

**Commit.** `docs(spec): sensitivity precision landed — divergence entries closed`

---

## Deferred during implementation

(Append-only.)

## Verification

- `cargo test -p smelt-cli --test maintenance_conformance` — including the new `ValueEnrichedRecipe` and column-add schedules
- `cargo test -p smelt-logical --test walk_coverage`, `cargo test -p smelt-runtime --test statement_parity`
- `bash .claude/scripts/verify-phase.sh`
- `/smelt:validate model_properties` and `/smelt:validate incremental_models` report zero drift on the touched sections
