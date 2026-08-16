# Phase 5 — generative definition-edit schedules

## Objective

Make the *generative* maintenance-conformance suite stage definition edits mid-history and assert
the new-definition oracle after every subsequent run step — today `ConformanceStep::RewriteModel`
exists but only hand-written cases in `gate.rs` ever construct one, so the pool never exercises a
definition change. Advances success criterion 4 (the generative half) and criterion 8 (the spec's
"the conformance gate covers definition edits" claim becomes true). The migrate-driven recovery
leg is phase 6.

## Spec delta

`docs/specs/definition_deltas.md` §"Known Divergences / Open Questions":

- Remove the bullet **"The conformance harness has no definition-edit step kind yet"** — the
  generative suite stages them after this phase, matching §Constraints' "the generative
  equivalence suite stages definition edits mid-history and asserts the new-definition oracle
  after every step" and §"The oracle".
- In the diagnostic-rename bullet, fix the stale tracking pointer `outcome.md` **phase 6 → phase
  8** (the 2026-08-17 reshape renumbered it).

No other spec or user-doc change: this phase adds test coverage, not user-visible behaviour.

## Tests

Red-green, in this order.

1. `smelt-maintenance-testkit` unit (`schedule_gen.rs`) —
   `definition_edit_schedule_stages_rewrite_then_recovery`: for an evolvable recipe the generated
   schedule contains exactly one `RewriteModel`, at least one `RunWindow` precedes it, and the
   step immediately following it is `FullRefreshRun`.
2. `smelt-maintenance-testkit` unit — `definition_edit_schedule_is_not_permutable`:
   `is_permutable` returns `false` for any schedule containing a `RewriteModel` step (a rewrite is
   order-dependent by construction, like `AppendLateRow`/`DropStateDir`).
3. `smelt-maintenance-testkit` unit — `definition_edit_schedule_draws_only_applicable_edits`: the
   drawn `ModelEdit` is always a member of `recipe.evolution` (never `AddGroupingColumn` for a
   row-shaped construct, which `render_model_body_with_edit` panics on).
4. `gate.rs` — `definition_edit_pool_upholds_equivalence`: the standing generative gate.
   Deterministic seed, `case_count()` cases over `RecipePool::partition_append_only()`, admitted
   recipes only; drive the definition-edit schedule through `drive_and_assert` and assert
   S-restricted equivalence against the rewritten body's oracle after every run step. Asserts
   `rewritten_cases > 0` so a generator that silently stops emitting rewrites fails the gate.
5. `gate.rs` — `definition_edit_grouping_column_upholds_equivalence`: the `AddGroupingColumn`
   (skeleton-widening) leg over the aggregate constructs specifically, pinned to that edit rather
   than left to the draw.

## Tasks

1. `schedule_gen.rs`: add `RewriteModel` to `is_permutable`'s exclusion list (test 2).
2. `schedule_gen.rs`: add `arb_schedule_with_definition_edit(recipe: &ModelRecipe) -> impl
   Strategy<Value = ConformanceSchedule>` — build the base schedule via the existing
   `build_schedule`, then splice `RewriteModel { edit }` + `FullRefreshRun` at a generated index
   strictly after the first `RunWindow` and (for determinism of the settled point) before the
   trailing catch-up re-runs. `edit` is drawn from `recipe.evolution`; when that is empty the
   strategy yields the plain schedule unchanged. Add an `edit: Option<ModelEdit>` (or equivalent)
   parameterisation only if it keeps the two builders from duplicating window construction.
   Do **not** widen `arb_schedule_for` itself — `probes.rs`, `state_deletion.rs`,
   `contract_points.rs`, `harness_self_check.rs` and the Spark mirrors consume it and a rewrite
   would change their meaning.
3. `schedule_gen.rs`: refresh `ConformanceStep::RewriteModel`'s doc comment — its "that
   classification is unbuilt (no `derive_model_maintenance_plan` caller reads a prior definition)"
   paragraph is stale as of `smelt migrate` (phases 1–3). Say instead that this step asserts the
   run pipeline's own current-on-disk-SQL contract, and that the migrate-gated recovery path is
   separate (phase 6).
4. `gate.rs`: add the two gate tests (4, 5) next to the existing Phase-9 definition-change block
   (~line 4149), reusing `stage_recipe`/`classify`/`drive_and_assert`; the recovery step is
   `FullRefreshRun` for the reason that block's doc comment already records (a windowed re-run
   against a changed column shape hits a raw DuckDB column-count mismatch — the migrate path is
   phase 6).
5. Apply the spec delta above.
6. If the `AddGroupingColumn` leg fails: diagnose it as a real defect first (systematic debugging,
   not generator narrowing). Only if it is a genuine product gap may the generator be restricted —
   and then it gets a new outcome phase row plus a `definition_deltas.md` Known Divergences
   bullet, never a silent skip.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-maintenance-testkit --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb --quiet 2>&1 | tail -40`
- Soak the new gate wider than the default case count:
  `SMELT_CONFORMANCE_CASES=40 cargo test -p smelt-cli --test maintenance_conformance --features duckdb definition_edit 2>&1 | tail -20`
- `cargo check -p smelt-cli --tests --features spark 2>&1 | tail -20` (the Spark mirror consumes
  `schedule_gen`; it must still compile even though the suite is gated).

## Commit message

`test(conformance): stage definition edits in generated schedules and assert the new-definition oracle`
