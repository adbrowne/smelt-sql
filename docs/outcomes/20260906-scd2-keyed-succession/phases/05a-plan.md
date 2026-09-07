# Phase 5a plan — emitter inputs, derived purely

## Objective

Close the gap phase 4's summary named: the succession emitters take a row-local projection,
`{lead}`/`{lag}` templates, a payload-column list and a delete-flag expression that nothing in
the codebase derives yet. Derive all of it in `smelt-logical` — the classifier already holds
this material and discards it — and carry it to consumers on the plan result, so phase 5b's
runtime driver never re-parses model SQL. Advances criteria 4 (the emitters become callable
from a real plan) and 5 (its prerequisite), and keeps the maintenance-plan purity rule intact.

## Spec delta

None. No user-visible behaviour changes: this phase adds no diagnostic, no SQL a run emits, and
no surface. `incremental_shapes.md` §"The succession grain" already specifies the shape being
derived; the classifier rules in `model_properties.md` §"Keyed-succession classification" are
unchanged (only what the verdict *carries* widens, which the spec does not enumerate).

## Tests

Red-green, in `crates/smelt-logical/src/analysis/succession/tests.rs` (verdict widening) and a
new `crates/smelt-logical/src/maintenance/succession.rs` test module (recipe assembly):

1. `verdict_carries_row_local_projection` — a model projecting `customer_id, changed_at,
   UPPER(region) AS region` yields `row_local` = the three `(alias, source-expression)` pairs in
   the model's own column order, expression text verbatim from the CST.
2. `verdict_carries_lead_template` — `LEAD(changed_at) OVER (…) AS valid_to` yields
   `lead_derived == [("valid_to", "{lead}")]`.
3. `verdict_carries_wrapped_lead_template` — `LEAD(changed_at) OVER (…) IS NULL AS is_current`
   yields `("is_current", "{lead} IS NULL")` — the window call's own text range is the only part
   replaced; the wrapper text around it survives byte-for-byte.
4. `verdict_carries_lag_template` — the `LAG` counterpart lands in `lag_derived` with `{lag}`.
5. `verdict_carries_delete_flag_expression` — `QUALIFY NOT is_deleted` yields
   `delete_flag_expr == Some("is_deleted")`; a model with no `QUALIFY` yields `None`.
6. `recipe_payload_columns_exclude_key_clock_and_derived` — `SuccessionRecipe::from_verdict`
   over a verdict with key `customer_id`, clock `changed_at`, derived `valid_to`/`is_current`,
   row-local `region`/`is_deleted` puts exactly `["region", "is_deleted"]` in `payload_columns`.
7. `recipe_feeds_emitters_end_to_end` — a DuckDB-executed test in
   `crates/smelt-logical/tests/succession_emit.rs`: build the recipe from a *classified* model
   (not hand-written emitter arguments), drive `emit_succession_event_delta` +
   `emit_succession_patch` through it, and assert multiset equality against the model's own
   `LEAD` SQL at full refresh. This is the leg that proves the derivation, not just its shape.
8. `plan_result_carries_recipe_for_recognized_model` — in
   `crates/smelt-db/src/queries/maintenance/tests.rs`: `derive_model_maintenance_plan` over the
   running succession example returns `succession_recipe: Some(_)` whose key/clock/payload
   columns match the model; a `NotSuccession` model returns `None`.
9. `advisory_only_model_still_yields_recipe` — the `SuccessionPreFilterNegatesFlag` advisory
   does not suppress or alter the recipe (the advisory never changes admission).

## Tasks

1. Widen `SuccessionVerdict::Recognized` with `row_local: Vec<(String, String)>`,
   `lead_derived: Vec<(String, String)>`, `lag_derived: Vec<(String, String)>`, and
   `delete_flag_expr: Option<String>`. Keep `lead_cols`/`lag_cols`/`delete_flag` (existing
   consumers and criterion 1's wording read them) — the new fields are additive.
2. In `analysis/succession/mod.rs`, capture the row-local `(alias, expr.text())` pairs where the
   projection loop already classifies them, in projection order.
3. In `analysis/succession/window.rs`, extend `record_window` (or a sibling) to build the
   template: take the projected item's expression text and the window call's `TextRange`, and
   splice `{lead}`/`{lag}` over the window call's own span, offsetting by the item expression's
   range start. Refuse nothing new — every shape reaching here is already validated.
4. Set `delete_flag_expr` from the `QUALIFY NOT <flag>` operand's expression text (today only the
   column *name* is kept).
5. Add `pub struct SuccessionRecipe` + `from_verdict` to
   `crates/smelt-logical/src/maintenance/succession.rs`: fields `source_table`, `pre_filter`,
   `key_cols`, `clock_col`, `payload_columns`, `row_local_projection`, `lead_derived`,
   `lag_derived`, `delete_flag_expr` — every argument the four phase-4 emitters take, less the
   caller-supplied window predicate, presented table and dialect. `payload_columns` = row-local
   aliases minus key columns, clock column and derived aliases, in projection order.
6. Return it from `derive_succession_plan` on `SuccessionDerivation` (`recipe: Option<…>`,
   `None` on `NotSuccession`).
7. Add `pub succession_recipe: Option<SuccessionRecipe>` to
   `smelt-db`'s `MaintenancePlanResult`, populated in the `resolved_grain()`-is-`None` branch of
   `plan.rs`; every other construction site sets `None`.
8. Doc-comment the recipe as the single owner of the emitters' inputs, citing
   `CLAUDE.md` §"Maintenance-plan purity" — a consumer takes the recipe, never model SQL.

## Verification

- `cargo test -p smelt-logical --test succession_emit`
- `cargo test -p smelt-logical --lib succession`
- `cargo test -p smelt-logical --test walk_coverage` (leaf classification unchanged)
- `cargo test -p smelt-db --test integration` (plan derivation)
- `cargo test -p smelt-runtime --test statement_parity` (no-authoring leg must stay green)
- `bash .claude/scripts/verify-phase.sh`
- `bash .claude/scripts/large-file-check.sh` — report, do not `--update`, unless the summary
  carries the sign-off note.

## Commit message

`feat(smelt-logical): derive succession emitter inputs from the classifier verdict`
