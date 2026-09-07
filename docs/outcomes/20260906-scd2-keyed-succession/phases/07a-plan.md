# Phase 7a plan — Testkit scaffolding for the succession family

## Objective

Give `crates/smelt-maintenance-testkit` the typed vocabulary a succession conformance case
needs: an arrival-partitioned, delete-flagged source shape, a `SuccessionRecipe` + renderer,
the model-SQL full-refresh oracle, and a `families/gate_succession.rs` stage/insert/drive/assert
quartet — proven end to end by one smoke case in `crates/smelt-cli/tests/maintenance_conformance`.
Advances success criterion 6 (its scaffolding half); it is also the hard prerequisite for
phase 7b's leg matrix and phase 6's late-append probe leg.

## Spec delta

None. This phase adds test infrastructure only; no user-visible behaviour changes.

## Tests

Red-green, in this order:

1. `recipe::arrival_partitioned_source_has_distinct_partition_and_event_time_columns` —
   the new `SourceRecipe` constructor yields `partition_column != clock_column` and a
   `NOT NULL` `is_deleted` column.
2. `render::rendered_succession_source_declares_append_only_and_both_axes` — the rendered
   source YAML carries `mutation_profile: append_only`, `event_time_column: <t>`,
   `partition_column: <arrival>`, and the flag column typed `BOOLEAN NOT NULL`.
3. `render::rendered_succession_recipe_stages_cleanly` — mirrors
   `rendered_recipe_stages_cleanly`: a rendered `SuccessionRecipe` project parses, type-checks
   and stages with **zero** diagnostics (a rendered recipe that trips a diagnostic is a
   generator bug, never a skipped case).
4. `render::rendered_succession_model_is_classified_as_the_succession_grain` — the staged
   model's derived maintenance plan carries `Grain::Succession` + `Technique::SuccessionPatch`,
   not a refusal; asserted through the same path phase 3 wired.
5. `render::succession_oracle_body_is_the_model_sql_over_the_named_relation` —
   `render_succession_oracle_body_over(recipe, rel)` is the model's own SQL (including
   `QUALIFY NOT <flag>` and the clamp) with the source reference swapped for `rel`.
6. `succession::smoke_two_window_splice_matches_oracle` (new
   `crates/smelt-cli/tests/maintenance_conformance/succession.rs`) — two windows, the second
   inserting an event between two existing events of one key, driven through the real
   `execute_project` pipeline; end-state equivalence asserted after **every** window.
7. `succession::smoke_lag_projection_matches_oracle` — the same schedule with a `LAG`-projecting
   recipe variant, proving the renderer's `LAG` arm is exercised by the quartet, not just the
   `LEAD` arm.

## Tasks

1. Widen `SourceRecipe` (`crates/smelt-maintenance-testkit/src/recipe.rs`) with an
   arrival-partitioned constructor: `partition_column: Option<String>` (defaulting to
   `clock_column`, so every existing recipe is unchanged) plus a
   `delete_flag_column: Option<String>`; add `SourceRecipe::succession_events(...)`.
2. Add the typed `SuccessionRecipe { model_name, source, projection, lead_cols, lag_cols,
   clamp: Option<..>, delete_filter: bool }` next to `KeyedRecipe`, with a small set of named
   constructors (no proptest strategy yet — 7b adds the generated pool).
3. Add `render_succession_source_file`, `render_succession_model_file`,
   `render_succession_oracle_body_over`, and `stage_succession_for_target` to `render.rs`,
   following the `render_keyed_*` / `stage_keyed_for_target` shapes exactly.
4. Add `crates/smelt-maintenance-testkit/src/families/gate_succession.rs` — the
   `stage_succession_recipe_for` / `insert_row_succession_for` /
   `assert_succession_equivalence_for` / `drive_succession_and_assert_for` quartet, modelled
   on `gate_keyed.rs`; register in `families/mod.rs`. DuckDB only (Spark/BigQuery take the
   recorded downgrade — outcome §Out of scope), so the target arm refuses non-DuckDB rather
   than silently passing.
5. Extend `STracker`/schedule plumbing only as far as the smoke case needs — the oracle is the
   model's own SQL over the retained source relation, so reuse the existing `oracle_relation`
   seam rather than introducing a second comparator.
6. Add `succession.rs` to `crates/smelt-cli/tests/maintenance_conformance/` with tests 6 and 7,
   and register the module in that suite's `main.rs`.
7. Re-run the file-selection-sensitive gates (`walk_coverage`, `hardening_budget`) since this
   phase adds new files under both a testkit crate and a test directory; keep any new
   `unwrap`/`expect` out of counted production code (the testkit is a test-support crate, but
   `render.rs` is large — confirm with `bash .claude/scripts/hardening-budget.sh`).
8. If `render.rs` or `recipe.rs` crosses the large-file baseline, split the new succession
   material into its own module (`render/succession.rs`, `recipe/succession.rs`) in this phase
   rather than leaving it to the loop's shrink step.

## Verification

- `cargo test -p smelt-maintenance-testkit --quiet 2>&1 | tail -40`
- `cargo test -p smelt-cli --test maintenance_conformance succession --quiet 2>&1 | tail -40`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet 2>&1 | tail -40` (the seeded
  sample must stay green — no existing family may regress from the `SourceRecipe` widening)
- `bash .claude/scripts/hardening-budget.sh`
- `bash .claude/scripts/large-file-check.sh`
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`test(maintenance-testkit): add the arrival-partitioned succession recipe, renderer and family quartet`
