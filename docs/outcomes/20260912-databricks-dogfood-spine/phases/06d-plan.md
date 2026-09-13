# Phase 6d plan — a source-written cast target gets its per-dialect spelling

## Objective

Close the one defect standing between the model set and a clean full refresh on Databricks:
`CAST(x AS VARCHAR)` written in a model's own SQL prints verbatim, and Spark/Unity Catalog
rejects a bare `VARCHAR` cast target with `[DATATYPE_MISSING_SIZE]`. Route the printer's
`TYPE_SPEC` through the *existing* single owner of per-dialect type spelling
(`smelt-dialect/src/type_conformance.rs`, which already maps a bare string type to `STRING` on
SparkSQL for the cast-wrap), then re-run the live full refresh to 16/16. Advances criterion 6
(the whole model set completes a full refresh) and unblocks criteria 7 and 8, which would
otherwise be evaluated over 12 of 16 models.

## Spec delta

`docs/specs/multi_backend.md` §"Output-schema type conformance" — add a paragraph stating that
the per-dialect cast-target spelling applies to **every** cast smelt prints, the cast-wrap and a
model-authored cast alike, and name the rule: on the SparkSQL dialect an unqualified string
cast target (`VARCHAR` with no length, `TEXT`) prints as `STRING`, because Spark treats bare
`VARCHAR` as the read-compat char/varchar family and refuses it as a cast target. A length-
qualified `VARCHAR(n)` is unchanged. This is user-visible emission behaviour, so the spec edit
lands first.

## Tests

Red-green, in this order:

1. `type_conformance::tests::source_cast_spelling_spark_bare_varchar_is_string` — the new pure
   fn maps `"VARCHAR"` and `"TEXT"` to `Some("STRING")` on `SqlDialect::SparkSQL`.
2. `type_conformance::tests::source_cast_spelling_keeps_length_qualified_varchar` —
   `"VARCHAR(10)"` on SparkSQL returns `None` (emit source text verbatim).
3. `type_conformance::tests::source_cast_spelling_duckdb_never_rewrites` — every type name in
   the fixture list returns `None` on DuckDB, so DuckDB output stays byte-identical.
4. `type_conformance::tests::source_cast_spelling_unparseable_type_passes_through` — a type
   name `parse_type` rejects returns `None` rather than panicking or guessing.
5. `type_conformance::tests::source_cast_spelling_bigquery_rejected_families` — `VARCHAR` →
   `STRING`, `DOUBLE` → `FLOAT64` on BigQuery, matching what `type_cast_sql` already does.
6. `printer::tests::cast_target_spelling_is_dialect_aware` — `SELECT CAST(a AS VARCHAR) FROM t`
   prints `CAST(a AS STRING)` on SparkSQL and `CAST(a AS VARCHAR)` on DuckDB.
7. `printer::tests::double_colon_cast_target_spelling_is_dialect_aware` — `a::VARCHAR` prints
   `CAST(a AS STRING)` on SparkSQL (the `::` rewrite path reaches the same owner).
8. `crates/smelt-cli/tests/github_activity_databricks.rs::actor_sessions_compiles_without_bare_varchar`
   — compiling `silver.actor_sessions` for the databricks target yields SQL containing no
   ` AS VARCHAR)`; no workspace needed.
9. Existing gates that must stay green unchanged: `cargo test -p smelt-dialect --test
   emission_ownership` (the new spelling lives outside `src/printer/`, so the printer gains no
   `SqlDialect` variant branch — it passes `*ctx.dialect` to the owner) and `cargo test -p
   smelt-runtime --test projection_dialect_invariance`.

## Tasks

1. Write the spec paragraph in `docs/specs/multi_backend.md` §"Output-schema type conformance".
2. In `crates/smelt-dialect/src/type_conformance.rs`, add
   `pub fn source_cast_type_sql(type_text: &str, dialect: SqlDialect) -> Option<String>`:
   parse `type_text` with `smelt_types::parse_type`; on success compute
   `type_cast_sql(&dt, dialect)` and return `Some(spelling)` **only** when it differs from
   `dt.to_backend_sql()` (the canonical spelling the source text already stands for); return
   `None` on parse failure or no difference. Doc-comment it as the single owner shared with
   `wrap_with_type_casts`.
3. Add tests 1–5 alongside it.
4. In `crates/smelt-dialect/src/printer/mod.rs`, dispatch `SyntaxKind::TYPE_SPEC` in
   `print_node` to a small helper in `printer/rewrites.rs` that calls
   `source_cast_type_sql(<trimmed node text>, *ctx.dialect)` and emits the substitute when
   `Some`, else falls through to `print_children` verbatim (preserving leading/trailing
   trivia exactly — the `::` rewrite path already depends on trailing whitespace placement).
5. Add tests 6–7; run `cargo test -p smelt-dialect --quiet` and confirm no golden-output
   regression in `printer/tests.rs` (line 331's DuckDB assertion must be untouched).
6. Add test 8 in `crates/smelt-cli/tests/github_activity_databricks.rs`.
7. Live: `source scripts/dbx-dogfood-env.sh` and re-run the exact 6c command —
   `smelt run --target databricks --full-refresh --allow-full-refresh --event-time-start
   2026-08-05 --event-time-end 2026-08-07` — recording the run id and `outcome_counts`. If
   `dbx-dogfood-env.sh` cannot reach the workspace, emit `<<PHASE_BLOCKED>>` rather than
   claiming the offline half closes the row.
8. Write `phases/06d-summary.md`: the live counts, and every failure that remains quoted with
   its model and statement (recorded, not fixed — the outcome's rule), as row 7's baseline.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, shellcheck, full
  `cargo test`, `example_diagnostics`) — no ratchet lowered.
- `cargo test -p smelt-dialect --test emission_ownership --test template_emission --quiet`
- `cargo test -p smelt-runtime --test projection_dialect_invariance --test dialect_seam --quiet`
- `cargo test -p smelt-cli --features databricks --test github_activity_databricks --quiet`
- Live: the full-refresh run above, target 16 success / 0 failed / 0 skipped.

## Commit message

`outcome(databricks-dogfood-spine): phase 6d spells source cast targets per dialect and lands a 16/16 full refresh`
