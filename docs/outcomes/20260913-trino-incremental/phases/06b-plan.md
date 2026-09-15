# Phase 6b plan — the degraded families' emission residue

## Objective

Close criterion 5's per-family parity for the one family it still misses: the additive keyed
fold's **downgrade** route, whose whole-target rebuild is proved today only by result-equality
(3h's summary). Route that rebuild through the single-owner emitter so its executed SQL is
recordable, and add its byte-identity leg on both DuckDB and the live Trino tier. Then settle
succession's partition-literal residue — the `driving_steps` call site in `execute/project/mod.rs`
that still passes `PartitionColumnType::Undeclared` (3f's untouched fourth site) — by measurement
rather than by assuming 3b2's renderer applies.

## Spec delta

None. The statement *text* does not change (`CREATE TABLE {schema}.{table} AS {sql}` either way);
what changes is who authors it, which `docs/specs/incremental_models.md` §"Statement emission
(single owner)" already requires. No user-visible behaviour moves, so no spec edit precedes this
phase.

## Tests (red-green)

1. `statement_parity/region_and_keyed_fold.rs::additive_keyed_fold_downgrade_statements_come_from_the_emitter`
   (DuckDB, always runs) — a `SUM` fold under `state.warehouse_tables: none` run through
   `execute_project` records exactly one `StatementGroup`, byte-identical to a direct
   `emit_create_table_as(&format!("{schema}.{table}"), &compiled_sql, MaintenanceDialect::DuckDb)`.
   Red today: the route calls `Backend::create_table_as`, so nothing is recorded at all.
2. `statement_parity/trino.rs::additive_keyed_fold_downgrade_parity_on_trino` (live-gated) — the
   same shape over a real `TrinoBackend`: exactly one recorded group, no `MERGE INTO` anywhere in
   the recording, byte-identical to `emit_create_table_as(..., MaintenanceDialect::Trino)`.
3. `crates/smelt-runtime/tests/succession_literal_census.rs::succession_renders_its_window_only_through_the_untyped_predicate`
   — source-scan census over `crates/smelt-runtime/src/maintenance_driver/succession/`: no
   `partition_literal(` call and no `PartitionColumnType` consumption outside comments, so
   `succession_window_predicate`'s deliberately untyped spelling is the family's only window
   rendering (cross-referenced to `maintenance_sql_dialect_purity.rs::the_succession_window_predicate_uses_untyped_date_literals`).
4. `crates/smelt-runtime/tests/succession_literal_census.rs::succession_driving_steps_column_type_is_inert`
   — `driving_steps(start, end, Day, Date)` and `driving_steps(start, end, Day, Undeclared)` yield
   steps whose `partition_value`/`range.start`/`range.end` are equal, and
   `succession_window_predicate` over each yields the same string: the `Undeclared` at the call
   site changes no emitted SQL, so it is residue by construction rather than by luck.
5. `crates/smelt-logical/tests/maintenance_availability.rs::succession_patch_always_downgrades_on_trino`
   — a `Technique::SuccessionPatch` cell resolved for `SqlDialect::Trino` always carries a
   `state_downgrade` (the tombstone ledger is unrealisable there), so the window-forward patch arm
   — and with it the `driving_steps` call site — is unreachable on Trino. This is the measurement
   the row asks for, made offline and permanently, rather than only observed once live.

## Tasks

1. In `crates/smelt-runtime/src/execute/project/mod.rs`, the `keyed_fold_state_downgrade` arm
   (~L1726): keep `backend.drop_table_if_exists` (a bare `DROP` is a table-lifecycle helper, not a
   maintenance statement — `structural_and_ledger.rs`'s own scan comment says so) and replace
   `backend.create_table_as(...)` with
   `backend.execute_statement_group(&emit_create_table_as(&format!("{schema}.{db_table_name}"), &compiled.sql, smelt_backend::maintenance_dialect(backend.dialect())?))`,
   matching `maintenance_driver/driver.rs`'s existing bare `schema.table` convention.
2. Write test 1 (red), then confirm green. Verify `keyed_fold_state_downgrade_execution.rs` and
   `dry_run_statements` still pass unchanged — the statement text must be identical to before.
3. Parameterise `statement_parity/trino.rs::stage_keyed_fold_project` over the combiner
   (`MIN`/`SUM`), the way `smelt-cli`'s twin already is, and write test 2.
4. Add `crates/smelt-runtime/tests/succession_literal_census.rs` with tests 3 and 4; give the
   `driving_steps(..., Undeclared)` call site a doc comment stating *why* `Undeclared` is correct
   there (succession consumes only `step.range.{start,end}` as strings) and pointing at the census.
5. Add test 5 in `smelt-logical`.
6. Update `statement_parity/main.rs`'s module doc and `trino.rs`'s header to name the downgrade
   leg; re-run `trino_ci_wiring.rs` (no new live-gated binary is introduced, so its census should
   need no widening — confirm rather than assume).

## Risks to measure, not assume

- The emitter's bare `schema.table` must resolve on Trino against the session's default catalog.
  3h's `MERGE INTO {schema}.device_agg` already proves this shape executes there; if the rebuild
  nonetheless fails, prefer fixing the call site's qualification over re-introducing backend
  authoring.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --test succession_literal_census --test maintenance_sql_dialect_purity --test keyed_fold_state_downgrade_execution --test dry_run_statements`
- `cargo test -p smelt-logical --test maintenance_availability --test walk_coverage`
- `cargo test -p smelt-cli --test trino_ci_wiring`
- Live tier (`bash scripts/trino-up.sh` / `source scripts/trino-env.sh` / `bash scripts/trino-down.sh`):
  `cargo test -p smelt-runtime --test statement_parity -- --test-threads=1`. If the coordinator is
  unreachable, emit `<<PHASE_BLOCKED>>` — never let the live leg skip green.

## Commit message

`feat(trino): emit the additive fold's downgraded rebuild from the single owner, and pin succession's window literal as untyped residue`
