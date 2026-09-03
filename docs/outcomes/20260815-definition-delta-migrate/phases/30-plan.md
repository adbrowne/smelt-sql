# Phase 30 plan — backbuild family joins the `statement_parity` gate

## Objective

Extend `crates/smelt-runtime/tests/statement_parity.rs` so the backbuild emitter family
(`crates/smelt-logical/src/backbuild/emit.rs`) is covered by the same two legs the maintenance
families already have: executed SQL byte-identical to a direct emitter call over the same inputs,
and a result-equivalence check against a full refresh. Widen the structural no-authoring leg's
*shape* list to the backbuild statement families. Serves success criterion 9 (standing gates green
for this outcome's own mechanism) and closes `architecture.md`'s narrowed Known Divergences bullet.

## Spec delta

`docs/specs/architecture.md`:

- **Delete** the Known Divergences bullet beginning "**Backbuild's executed-statement parity leg is
  proven end to end but not yet by the `statement_parity` structural gate itself.**" (line ~523).
- In §"Constraints & Invariants" item 12, extend the closing "Standing CI gate" sentence so it
  states the parity diff covers the backbuild migration families (`ALTER TABLE`, in-place `UPDATE`,
  full-refresh `CREATE OR REPLACE TABLE`) alongside the maintenance families, and that the
  structural leg's forbidden-shape list covers both. Keep it timeless — no phase vocabulary.

No user-visible behaviour changes, so no docs-site edit.

## Tests

New in `crates/smelt-runtime/tests/statement_parity.rs` (a `RecordingBackend` + real DuckDB, driving
`smelt_runtime::definition_delta::{derive_plan, apply_migration}` directly — the same "drive the
single dispatch point" rationale `recurrence_bound_probe_and_checked_merge_come_from_the_emitters`
already documents):

1. `backbuild_in_place_backfill_statements_come_from_the_emitter` — a definition edit adding a
   column derivable from the model's own stored columns: executed SQL == `emit_alter_add_column` +
   `emit_in_place_update` called directly over the derived `BackbuildInputs`, byte for byte, and the
   migrated table is `multiset_equal` to a full refresh of the after-definition.
2. `backbuild_full_refresh_statement_comes_from_the_emitter` — a skeleton-changing (grain-changing)
   edit whose plan is the single `emit_full_refresh` statement; same two assertions.
3. `backbuild_upstream_backfill_statements_come_from_the_emitter` — a column derived from an
   upstream model, exercising the `emit_column_backfill_update_from` (or
   `…_from_subquery`, whichever the classifier admits for the fixture — name the admitted technique
   in the test doc comment); same two assertions.
4. `no_maintenance_statement_authoring_outside_the_emitter` (existing, extended) — the forbidden
   shape list gains the backbuild families. Must go red first against the un-allowlisted tree, then
   green.

## Tasks

1. Read `crates/smelt-logical/src/backbuild/classify.rs`'s admission conditions for the three
   techniques above so each fixture edit lands on the intended verdict; assert the derived
   `plan.groups` verdict/technique in each test rather than trusting the statement text alone.
2. Add a shared helper in the test file that stages a project, deploys v1 through
   `execute_project`, rewrites the model file, and returns `(DerivedPlan, RecordingBackend)` —
   the three legs differ only in fixture SQL.
3. Write test 1 (red → green), then 2, then 3.
4. Extend `scan_statement_authoring_file`'s forbidden shapes with the backbuild families:
   `"ALTER TABLE "`, `"CREATE OR REPLACE TABLE "`, and a shape distinctive to the in-place
   `UPDATE`/difference-`INSERT` emitters (pick one with no legitimate production match; document
   why each shape is unambiguous in the function's doc comment, as the existing
   `CREATE TEMP TABLE ` note does).
5. Run the widened scan. Every surviving hit inside the currently-scanned crates
   (`smelt-backend*`, `smelt-backends`, `smelt-runtime`, `smelt-logical`) is either fixed by routing
   through the emitter or added to `STATEMENT_AUTHORING_ALLOWLIST` with a one-paragraph
   justification. **Do not** add `smelt-state` to the scanned crate list — its `ddl_duckdb.rs`
   second author is phase 30b's scope; record what you saw in the summary for that planner.
6. Update the test file's `//!` header to name the backbuild family and its emitter module.
7. Apply the spec delta above.

## Verification

- `cargo test -p smelt-runtime --test statement_parity` — all legs, old and new.
- `cargo test -p smelt-cli --test migrate_apply` — the pre-existing end-to-end migrate path is
  unaffected.
- `cargo test -p smelt-logical --test walk_coverage` — the emitter module's tags still hold.
- `rg -n "statement_parity structural gate itself|not yet by the .statement_parity" docs/specs` —
  no hits (bullet removed).
- `bash .claude/scripts/verify-phase.sh` — must be ALL GREEN.

## Commit message

`test(statement-parity): cover the backbuild emitter family in both legs`
