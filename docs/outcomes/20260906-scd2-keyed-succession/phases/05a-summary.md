# Phase 5a summary — emitter inputs, derived purely

## Shipped

- `SuccessionVerdict::Recognized` (`crates/smelt-logical/src/analysis/succession/mod.rs`)
  widened with `row_local`, `lead_derived`, `lag_derived`, `delete_flag_expr` — the
  classifier's own expression material, captured at classification time instead of
  discarded.
- `window::derived_template` (`analysis/succession/window.rs`) splices the literal
  `{lead}`/`{lag}` token over a window call's own span inside the select item's full
  expression text — correct for both a bare projection and a scalar-wrapped one
  (`LEAD(t) OVER (...) IS NULL`).
- `SuccessionRecipe` + `SuccessionRecipe::from_verdict`
  (`crates/smelt-logical/src/maintenance/succession.rs`) — the single assembler that
  turns a `Recognized` verdict into every argument the phase-4 emitters take
  (`source_table`, `pre_filter`, `key_cols`, `clock_col`, `payload_columns`,
  `row_local_projection`, `lead_derived`, `lag_derived`, `delete_flag_expr`).
  `payload_columns` = row-local aliases minus key/clock/derived aliases.
- `SuccessionDerivation.recipe: Option<SuccessionRecipe>`, `None` on `NotSuccession`.
- `smelt-db`'s `MaintenancePlanResult.succession_recipe`, populated on the
  `resolved_grain()`-is-`None` branch of `derive_model_maintenance_plan`, `None`
  everywhere else (every other `MaintenancePlanResult` construction site updated).
- `crates/smelt-logical/tests/succession_emit.rs::recipe_feeds_emitters_end_to_end` —
  builds the recipe from a *classified* model (not hand-written emitter args), drives
  the emitters through it against a real DuckDB, and matches the model's own `LEAD` SQL
  at full refresh.

## Decisions

- Boxed `row_local`, `lead_derived`, `lag_derived`, `delete_flag_expr` on the
  `Recognized` variant (`clippy::large_enum_variant` — `NotSuccession` carries only a
  reason string, so the unboxed variant was ~4x its sibling). Not a spec-visible
  change: `SuccessionRecipe`'s own fields stay unboxed plain `Vec`/`Option`, so every
  consumer outside this classifier reads ordinary types.
- `derived_template` computes the window call's own span as `func.start()` through
  `window_range.end()` (a field added to `WindowCall` since `smelt_parser::WindowSpec`
  exposes no `syntax()` accessor), then trims the spliced result — the parser's
  select-item expression text carries a trailing space before the `AS alias` token.

## For the next planner

- `SuccessionRecipe::source_table` is the classifier's raw comparison spelling
  (`ctx.source_name`, e.g. `sources.customer_changes` or a bare name depending on
  caller), not a physically resolved table name — phase 5b's runtime driver still owns
  resolving it to what the emitters' `source_table` argument should actually print.
- Everything in criterion 5 that needs the recipe (transactional ledger write +
  presented `MERGE`, clock-tie probe → rollback, refold/either-order convergence
  through the real driver, `execute_parity`) is still open — this phase only proves the
  recipe is derivable and emitter-shaped, not that the runtime dispatches it.
- The large-file ratchet is red on six files this phase's diff grew (plus several
  1-3-line drifts elsewhere from earlier phases, already present before this diff) —
  same non-blocking shape as phases 2b/3/3a, left to the loop's dedicated shrink step.

## Gates

- `cargo test -p smelt-logical --test succession_emit` — 7 passed
- `cargo test -p smelt-logical --lib succession` — 67 passed
- `cargo test -p smelt-logical --test walk_coverage` — 14 passed
- `cargo test -p smelt-db --test integration` — 370 passed
- `cargo test -p smelt-runtime --test statement_parity` — 37 passed
- `cargo test -p smelt-db --lib plan_result_carries_recipe_for_recognized_model` — 1 passed
- `bash .claude/scripts/verify-phase.sh` — fmt PASS, clippy (both feature sets) PASS,
  `example_diagnostics` PASS; `cargo test (workspace)` FAILs solely on
  `large_file_ratchet::gate_passes_on_committed_tree` (confirmed via
  `cargo test --workspace --no-fail-fast` — the only failing test in the whole
  workspace), recorded above rather than fixed, per plan and phase 2b/3/3a precedent.
- `bash .claude/scripts/large-file-check.sh` — reported (not `--update`d); six files
  from this phase's own diff plus pre-existing 1-3-line drift.
- `cargo test -p smelt-core --test hardening_budget` — production counts unchanged
  from baseline (the reported "regression" is the gate's own fixture probe, not real
  code).
