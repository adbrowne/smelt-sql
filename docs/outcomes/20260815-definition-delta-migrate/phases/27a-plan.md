# Phase 27a plan — `--show-sql` renders the suppressed form a run executes

## Objective

`smelt explain --show-sql` (and the identical `--json` `technique_previews` array the UI ships)
currently renders the **unconditional** matched arm for every cell, even where the report's own
"write variant: suppressed" line says a live run would execute the change-suppressed one. Close
that divergence by resolving write suppression in the preview builder through the same pure
`choice::` functions the driver uses, and emitting `emit_column_scoped_merge_suppressed` /
`emit_keyed_fold_suppressed` when the verdict is `Suppressed`. Serves the outcome's
"printed SQL cannot drift from executed SQL" criterion and removes the first clause of the
`Conditional-maintenance gaps` known-divergence bullet.

## Spec delta (make these edits first)

- `docs/specs/cli.md` §`--show-sql` — state that the rendered statements are the form a run
  would execute for that cell, including the change-suppressed matched arm wherever the cell's
  P2/P3 write-suppression proof (and the override ladder) admits it; a cell whose pin makes the
  variant unresolvable renders no statements for that technique rather than a plain arm.
- `docs/specs/ui_model_diagnostics.md` §Semantics "Technique preview set" — one sentence: a
  preview's statements carry the cell's resolved write variant, not a canonical unconditional
  rendering.
- `docs/specs/incremental_models.md` §Known Divergences "Conditional-maintenance gaps" — drop
  the `smelt explain --show-sql renders the unconditional matched arm…` clause only; leave the
  other clauses (rows 27b–27e own them) and the non-DuckDB widened-scan clause intact.
- `docs-site/docs/reference/cli.md` (or whichever page documents `explain --show-sql`) — matching
  user-facing sentence.

## Tests (red first)

- `crates/smelt-runtime/tests/diagnostics.rs::column_scoped_merge_preview_renders_the_suppressed_matched_arm`
  — a suppressible `ColumnScopedMerge` cell's preview statements carry the
  `IS DISTINCT FROM` matched-arm guard.
- `…::keyed_fold_preview_renders_the_suppressed_matched_arm` — same for the keyed fold, whose
  guard compares the stored value against the fold's own combine expression.
- `…::first_build_cell_preview_keeps_the_unconditional_matched_arm` — a cell with
  `ledger_catch_up` / `Trigger::Backfill` (no prior state to diff) renders no guard, matching
  `resolve_write_variant`'s default.
- `…::incomparable_group_preview_keeps_the_unconditional_matched_arm` — a P3 refusal renders the
  plain arm.
- `…::suppress_pin_over_a_refused_proof_yields_no_preview_statements` — a `technique: suppress`
  pin whose proof refused surfaces as a build error (empty statements, non-`Admitted`
  admissibility), never a silent unconditional fallback.
- `crates/smelt-cli/tests/explain_model.rs::show_sql_renders_the_suppressed_form` — end-to-end:
  for a model whose report prints `write variant: suppressed`, the printed SQL block carries the
  guard.
- `crates/smelt-runtime/tests/statement_parity.rs::preview_guard_matches_executed_suppressed_merge_guard`
  — extract the matched-arm suppression predicate from the preview and from the recorded executed
  statement for the same model/cell and assert them byte-identical (mirrors the existing
  `observed_delta_predicate_matches_suppressed_merge_guard_byte_for_byte` shape).

## Tasks

1. Make the spec + docs-site edits above.
2. Add one pure resolver in `smelt-logical::maintenance::choice` —
   `resolve_cell_write_suppression(group_columns, sql, cell, overrides) -> Result<WriteSuppression, ChoiceRefusal>`
   — folding today's driver sequence (`model_property_vector(...).comparability` →
   `resolve_write_suppression` → `resolve_write_variant`) into one place.
3. Rewrite `maintenance_driver.rs`'s two inline copies (≈lines 1305–1330 and ≈1615–1640) to call
   it, asserting no behaviour change via the existing driver tests.
4. Thread the cell's column-group columns into the preview builder: add a `column_groups`
   parameter to `build_model_diagnostics` / `build_plan_cell_diagnostics`
   (`crates/smelt-runtime/src/diagnostics.rs`) and update callers —
   `smelt-cli/src/commands/explain.rs`, `smelt-ui/src/build.rs`, plus the three test call sites
   (`smelt-runtime/tests/diagnostics.rs`, `smelt-ui/tests/api.rs`).
5. In `build_technique_statements`, for `Technique::ColumnScopedMerge` and `Technique::KeyedFold`,
   call the new resolver with the model's frontmatter override ladder (`effective_override` over
   `model.metadata`) and emit the `*_suppressed` variant on `Suppressed`; propagate a
   `ChoiceRefusal` as `Err` so the preview reports non-admitted rather than falling back.
6. Confirm `explain.rs`'s existing "write variant: …" report line and the newly rendered SQL are
   driven by the same verdict (no second resolution left in `smelt-cli`).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test diagnostics --test statement_parity --test dry_run_statements`
- `cargo test -p smelt-cli --features duckdb --test explain_model --test explain`
- `cargo test -p smelt-ui --test api`
- `cargo test --workspace` (phase 25's summary: a shared-resolver change breaks tests outside the
  listed files; sweep before declaring green)

## Commit message

`feat(explain): render the change-suppressed matched arm in --show-sql previews`
