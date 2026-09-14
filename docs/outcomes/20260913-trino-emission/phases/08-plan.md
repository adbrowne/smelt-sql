# Phase 8 plan — Seams: refusal, ownership, projection invariance on a fourth dialect

## Objective

Widen the three standing seam gates to Trino so the criteria that say "on the compile path" and
"across four dialects" are mechanically true rather than asserted: `dialect_seam` gains Trino
refusal coverage including inside `smelt.define` function bodies (criterion 8),
`emission_ownership`'s dialect-branch gate stops restating a three-name list that silently omits
Trino (criterion 7), and `projection_dialect_invariance` compiles its every-lowered-construct
model for a fourth target and asserts byte-identical `output_columns` (criterion 9). Entirely
offline — no coordinator, no live leg; this phase is per-PR gate work.

## Spec delta

1. `docs/specs/multi_backend.md` §"Output-schema type conformance" — the paragraph stating the
   projection is derived from the pre-print CST gains one sentence naming the standing gate and
   its dialect set: the model exercising every lowered construct compiles for DuckDB, Spark SQL,
   BigQuery **and Trino**, with byte-identical output column names and cast-wrap names across all
   four.
2. `docs/specs/multi_backend.md` §"Refusal covers function bodies" — one sentence adding Trino's
   concrete instances to the existing rule: a one-argument `LOG(x)` (the registry's
   `Conditional` → `Unsupported` arity arm) and an `UNPIVOT` clause (the `supports_unpivot` =
   false clause-level refusal) are each refused at compile time when written inside a function
   body, not only in a model's own tree.
3. `CLAUDE.md`, the **Source-derived projection** bullet — "for DuckDB, Spark and BigQuery …
   byte-identical across all three" → four dialects including Trino.

No behaviour change: all three are statements of what the widened gates now prove.

## Tests

All red-then-green. Trino target fixture shape: `target_type: "trino"`, `catalog: Some("iceberg")`,
`host: Some("localhost")`, `user: Some("smelt")`, `schema: "main"` (see
`crates/smelt-core/src/config.rs::make_trino_target`); no connection is opened at compile time.

`crates/smelt-runtime/tests/dialect_seam/` (add `trino_target()` to `fixtures.rs`, register it in
`registry()`; tests in `refusals.rs`):

1. `one_argument_log_is_refused_for_trino` — `SELECT LOG(x) AS l FROM t` for the `trino` target
   fails with `UnsupportedOnBackend`, names `LOG`, and carries the registry's own reason text
   (the arity-1 `Conditional` arm), not a warehouse-shaped error.
2. `two_argument_log_compiles_for_trino` — the other arm of the same verdict compiles, so test 1
   is proving the arm and not a blanket refusal.
3. `unpivot_is_refused_for_trino_on_the_compile_path` — a model whose `FROM` carries an `UNPIVOT`
   clause refuses for `trino` with `UnsupportedOnBackend` naming `UNPIVOT`, and the same model
   compiles for `duckdb` (the refusal is the dialect's). Moves phase 4's `compile.rs` unit
   assertion onto the public seam suite.
4. `a_refused_construct_inside_a_function_body_is_refused_for_trino` — the phase-8 row's
   function-body leg: a `smelt.define` body containing one-argument `LOG(x)`, and a second body
   containing an `UNPIVOT` clause, are each refused for `trino` with `UnsupportedOnBackend`
   naming the construct. Modelled on the existing BigQuery loop test in the same file.
5. `floor_divide_compiles_for_trino` — the positive contrast with the existing BigQuery refusal:
   `//` is a stated `Template("{0} / {1}")` on Trino (phase 3), so it compiles and the printed
   SQL contains `/` and no `DIV`.

`crates/smelt-dialect/tests/emission_ownership.rs`:

6. `the_printer_branches_on_no_dialect_variant` — rewritten to derive the forbidden variant list
   by parsing `src/dialect.rs`'s `enum SqlDialect` body (same "parsed out of the module, not
   restated" shape `declared_rewrite_ids`/`declared_restructure_ids` already use) instead of the
   hardcoded three names; assert the derived list is non-empty and contains `Trino`, so an empty
   parse cannot make the gate vacuous.

`crates/smelt-runtime/tests/projection_dialect_invariance.rs` (add `trino_target()`, register it):

7. `output_columns_and_cast_wrap_names_are_byte_identical_across_backends` — widened to four
   backends; Trino's `output_columns` must equal DuckDB's and the source-derived expectation, and
   the cast-wrap / no-positional-fallback loops include it.
8. `decorrelated_model_output_columns_are_identical` — Trino added to all three cases; its
   projection matches, and (pinning phase 7's measured negative result) the `window_to_cte` case's
   Trino SQL carries no `__smelt_` synthesis, since Trino needs no restructure there.

If a construct in `EVERY_LOWERED_CONSTRUCT_SQL` turns out to refuse on Trino, that is a finding to
record in the summary and settle as a registry verdict — do **not** weaken the model to route
around it, and do not add a Trino-only expected-column list: four-way byte-identity is the point.

## Tasks

1. Write tests 1–5 red: add `trino_target()` to `dialect_seam/fixtures.rs` and register it.
2. Make them green — expected to need no production change (the verdicts and the clause refusal
   already ship); if one does, it is registry data or a `SqlDialect` fact, never a printer branch.
3. Rewrite test 6 to parse the `SqlDialect` enum; confirm it fails when a `SqlDialect::Trino`
   string is temporarily planted in a printer file, then remove the plant.
4. Add `trino_target()` to `projection_dialect_invariance.rs` and widen tests 7–8 to four dialects.
5. Land the three spec/CLAUDE.md edits from §Spec delta.
6. Run the gates; record any finding (a construct refusing unexpectedly on Trino) in
   `phases/08-summary.md` alongside the usual shipped/decisions/for-the-next-planner sections.

## Verification

- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --quiet`
- `cargo test -p smelt-dialect --test emission_ownership --quiet`
- `cargo test -p smelt-types --test registry_coverage --quiet` (verdicts untouched, must stay green)
- `bash .claude/scripts/verify-phase.sh` — run with the Trino tier **unexported** (phases 3–7's
  discipline: the shared Iceberg catalog collides under full-workspace concurrency)
- No ratchet moves in this phase: `dialect-gaps-baseline.txt`, `registry-migration-baseline.txt`,
  `parser-gaps-baseline.txt` and `hardening-baseline.txt` must all be untouched.

## Commit message

`test(dialect): widen the refusal, ownership and projection seams to Trino`
