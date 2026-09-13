# Phase 2 plan — `DialectId::Trino` + `SqlDialect::Trino` land exhaustively

## Objective

Land the two dialect-identity variants and resolve every arm they break — by the compiler
*and* by audit of the wildcard arms the compiler cannot flag. Advances criterion 2, and is
the unblocking prerequisite for phases 3–8. No arm added here may assert a positive claim
about Trino's SQL surface: each is either a fail-loud refusal, or a conservative value
documented as provisional with phase 8 named as its measuring phase.

## Spec delta

None. Phase 1 already specified the `trino` target, the `SqlDialect::Trino` dialect and the
implicit-`Native` emission Known Divergence. This phase is code-only against that spec.

## Tests

1. `smelt-types` `dialect_id::tests::all_is_exhaustive` — extend the exhaustive match and the
   length assertion to 4; red until `ALL` carries `Trino`.
2. `smelt-types` `dialect_id::tests::slug_round_trips_and_matches_the_existing_spelling` — assert
   `DialectId::Trino.slug() == "trino"` and that it round-trips through `from_slug`.
3. `smelt-dialect` `dialect::tests::every_sql_dialect_maps_to_a_distinct_dialect_id` — add
   `SqlDialect::Trino` to the array; the existing `sorted.len() == DialectId::ALL.len()`
   assertion is what proves no `DialectId` is left without a `SqlDialect`.
4. `smelt-dialect` new `dialect::tests::trino_dialect_identity_and_language_properties` —
   `name() == "Trino"`, `id() == DialectId::Trino`, and both language-property predicates are
   `false` (the conservative landing: a `false` makes smelt *refuse* the construct rather than
   emit SQL Trino may reject; phase 8 measures the real values).
5. `smelt-dialect` new `type_conformance::tests::trino_cast_spelling_is_a_named_arm` — asserts
   `type_cast_sql` routes Trino through an explicitly-named arm, pinning today's spellings
   (`Text`/`Varchar{None}` → `VARCHAR`) so phase 6/7's live run changes a *test*, not a silent
   wildcard. Red today: the `_ =>` fall-through absorbs Trino with no arm to assert against.
6. `smelt-state` new `ledger::tests::trino_ledger_statements_are_refused` (and the sibling
   assertions in `observed_delta.rs` / `tombstone.rs`) — every state-SQL entry point returns
   the typed `Unsupported*Dialect` error for `SqlDialect::Trino`, never a DuckDB/BigQuery
   spelling and never a panic.
7. `smelt-logical` new `state_structure` test — `realisable_state_structures(SqlDialect::Trino)`
   is empty, matching Spark's column per the 2026-09-13 ruling.
8. `smelt-backend` new `tests` — `maintenance_dialect(SqlDialect::Trino)` is
   `Err(UnsupportedMaintenanceDialect)` naming Trino, and the other three are `Ok`.
9. `smelt-runtime` new `compile::tests::as_struct_dialect_name_comes_from_the_slug` — the
   `dialect_name` used by the as-struct emitter equals `self.dialect.id().slug()` for all four
   dialects; red because `compile.rs` currently restates the three slugs in a second table.

## Tasks

1. `crates/smelt-types/src/dialect_id.rs`: add `DialectId::Trino`, extend `ALL`, add the
   `"trino"` slug arm; update tests 1–2.
2. `crates/smelt-dialect/src/dialect.rs`: add `SqlDialect::Trino`; `name()` → `"Trino"`;
   `id()` → `DialectId::Trino`; both language-property predicates → `false` with a doc comment
   stating the value is a conservative provisional landing measured in phase 8. Do **not** add
   a `BackendCapabilities::trino_iceberg()` constructor — phase 8 owns it.
3. `crates/smelt-dialect/src/type_conformance.rs`: give `type_cast_sql` an explicit
   `(dt, SqlDialect::Trino)` arm rather than letting `_ =>` absorb it. Keep today's
   `to_backend_sql()` behaviour as the arm body, with a doc comment naming the two spellings
   already suspect against Trino (`Float` → `FLOAT`, `Blob` → `BLOB`; Trino has `REAL` and
   `VARBINARY`) as phase 6/7's to measure.
4. `crates/smelt-state/src/{ledger,observed_delta,tombstone}.rs`: extend the 11 existing
   `SqlDialect::SparkSQL => Err(…)` refusal arms to `SqlDialect::SparkSQL | SqlDialect::Trino`.
   Widen each refusal's doc comment to say Iceberg shares Delta's per-table-commit atomicity,
   citing the 2026-09-13 ruling; `20260913-trino-ledger` revisits, this is not a deferral.
5. `crates/smelt-logical/src/maintenance/availability/state_structure.rs`: same widening to
   `vec![]`, with the prose extended to name Trino alongside Spark.
6. `crates/smelt-backend/src/lib.rs`: change `maintenance_dialect` to
   `Result<MaintenanceDialect, UnsupportedMaintenanceDialect>`, a new typed error mirroring
   `smelt_state::UnsupportedLedgerDialect`. Do **not** add a `MaintenanceDialect::Trino`
   variant — that would demand ~150 Trino SQL spellings across ten emitters, which is
   `20260913-trino-incremental`'s subject.
7. Thread the `Result` through every `maintenance_dialect` caller (~20 sites: `smelt-runtime`
   `profile.rs`, `execute/{project/mod.rs,key_addressed.rs,bootstrap.rs}`, `smelt-cli`
   `commands/explain.rs`, `smelt-ui` `build.rs`). All sit in `Result` contexts; the `smelt-ui`
   and `smelt-cli` `backend_type_to_maintenance_dialect` helpers become fallible too.
8. `crates/smelt-runtime/src/compile.rs:~1489`: replace the hand-restated `dialect_name` match
   with `self.dialect.id().slug()` — single ownership, and it removes an arm Trino would
   otherwise have to be added to twice.
9. `crates/smelt-runtime/src/schema_evolution.rs:~129`: name the Trino arm in
   `ddl_backend_for_dialect`. Pick the shape the implementer can justify from the existing
   `DdlBackend` enum; if none fits without inventing Trino DDL, make the function fallible in
   the same style as task 6 rather than aliasing Trino onto `DdlBackend::Spark`.
10. Wildcard audit: `rg -n '_ =>' ` over every `crates/*/src` and `crates/*/tests` file that
    mentions `SqlDialect` or `DialectId`, and confirm each catch-all either cannot be reached
    with a dialect scrutinee or is converted to a named arm. Record the audited list in the
    phase summary. (`Signature::engine_native.get(&dialect) -> Native` is the one *known,
    specified* implicit default — phase 1 recorded it as a Known Divergence owned by
    `20260913-trino-emission`; leave it and say so in the summary.)
11. Add the decision-log entries below to `outcome.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, shellcheck, full
  workspace test, example_diagnostics).
- `cargo test -p smelt-types --lib dialect_id`
- `cargo test -p smelt-dialect` (incl. `capability_conformance`, `emission_ownership`,
  `template_emission`, `operand_conditional`)
- `cargo test -p smelt-state --test ledger_dialect`
- `cargo test -p smelt-cli --test trino_spec_freshness` (phase 1's gate must stay green — in
  particular the Trino capability column must remain entirely `?`)
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --test statement_parity`
- `cargo test -p smelt-core --test hardening_budget` — no new `unwrap`/`expect` absorbed.

## Commit message

`feat(dialect): land DialectId::Trino and SqlDialect::Trino with every arm named`
