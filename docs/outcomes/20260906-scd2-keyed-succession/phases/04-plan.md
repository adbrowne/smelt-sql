# Phase 4 plan — succession emitters, tombstone-ledger DDL, and the append-only SCD2 conformance cell

## Objective

Make the succession-patch technique *executable text*: four pure emitters in `smelt-logical`
(event-delta `SELECT`, succession-patch `MERGE` over the neighbour domain, ledger-rebuild
`SELECT`, clock-tie probe) plus the per-model tombstone-ledger DDL in `smelt-state`, each proven
against a real DuckDB. Advances criterion 4 (emitters, ledger DDL, no-authoring leg) and the
matrix half of criterion 3 (the SCD2 append-only cell becomes an emitter-backed `CLAIMED`).
No runtime dispatch here — that is phase 5.

## Spec delta

None. `incremental_shapes.md` §"The tombstone ledger (hidden state)" (Physical shape, Lifecycle),
§"The maintenance theorem (bounded footprint)" and `model_transforms.md`'s succession row already
specify all three emitter outputs, the ledger's name/columns/PK, and the neighbour domain. This
phase implements what is written; no user-visible surface changes. (`model_transforms.md`'s
`unbuilt` → `built` flip belongs to phase 10's closure pass.)

## Tests

New `crates/smelt-logical/tests/succession_emit.rs` (a fresh file, not an extension of the
already-1710-line `emit_statements.rs`), text-shape legs first:

1. `tombstone_table_name_appends_the_reserved_suffix` — `<presented table>__tombstones`, schema
   qualification preserved.
2. `event_delta_select_projects_row_local_columns_and_the_delete_flag_with_no_window_function` —
   carries the pre-filter and the window predicate; contains no `OVER (`.
3. `patch_group_is_transactional_and_records_tombstones_before_the_presented_merge`.
4. `patch_merge_neighbour_domain_unions_presented_ledger_and_batch` — all three relations appear
   in the domain CTE; the `LEAD`/`LAG` recomputation runs over it, not over the presented table.
5. `patch_merge_keys_on_key_columns_and_the_clock` — `ON` is `(k…, t)`, so the write is
   idempotent on `(k, t)`.
6. `ledger_rebuild_select_is_key_and_clock_of_delete_flagged_rows_passing_the_pre_filter`.
7. `clock_tie_probe_selects_key_clock_and_a_sample_for_non_identical_collisions`.

DuckDB-executed legs (`duckdb::Connection::open_in_memory`, oracle = the model's own SQL with
`QUALIFY NOT <flag>` at full refresh, compared with the file's `multiset_equal` idiom):

8. `patch_matches_full_refresh_for_a_late_splice` — an event landing between two stored events
   patches predecessor and successor and takes its own derived columns from them.
9. `patch_matches_full_refresh_for_a_delete_then_a_later_insert` — proves the ledger is
   load-bearing: the same fixture with the ledger writes suppressed diverges from the oracle.
10. `patch_matches_full_refresh_for_a_lag_projecting_model` — `LAG`-derived columns patch the
    successor.
11. `refolding_a_window_leaves_table_and_ledger_unchanged` — byte-identical on the second apply.
12. `two_windows_applied_in_either_order_converge` — same final table and ledger.
13. `clock_tie_probe_fires_on_a_non_identical_collision_and_is_silent_on_a_redelivery`.

`crates/smelt-state/tests/` (or the module's own `#[cfg(test)]`, matching the file's convention):

14. `tombstone_table_ddl_declares_key_and_clock_not_null_with_primary_key` — columns exactly
    `key_cols ++ [clock_col]` in the model's own type spellings, `PRIMARY KEY (k…, t)`, and a
    matching drop.

`crates/smelt-logical/tests/maintenance_plan_conformance.rs`:

15. `described_technique_matches_execution_succession_patch` — derive the plan from a recognised
    verdict, emit, execute on DuckDB, `multiset_equal` against the model SQL at full refresh;
    inhabit the `SCD2 / versioned intervals` row's **column 0** (append-only) and add the
    matching `CLAIMED` entry naming this test. Columns 2 (EX-29, snapshot-derived) and 3 (EX-28,
    change feed) stay `KNOWN_GAPS` — both are out of this grain by construction. The existing
    `coverage_matrix_is_inhabited` and `claimed_and_known_gaps_partition_the_inhabited_cell_count`
    must stay green.

`crates/smelt-runtime/tests/statement_parity/structural_and_ledger.rs`:

16. `no_maintenance_statement_authoring_outside_the_emitter` extended with the succession shapes
    (`__tombstones`, the succession patch `MERGE`, the ledger-rebuild `SELECT`) — none constructed
    in `smelt-runtime/src` or `smelt-backend*/src`. Structural only; the executed-vs-emitted
    succession family leg needs the phase-5 driver and is planned there.

## Tasks

1. Add `crates/smelt-logical/src/maintenance/emit/succession.rs`; re-export from `emit/mod.rs`.
2. `tombstone_table_name(presented_table) -> String` — the single owner of the `__tombstones`
   suffix (`smelt-state` does not depend on `smelt-logical`, so its DDL takes the derived name as
   a parameter rather than re-deriving it).
3. `emit_succession_event_delta` — pre-filter + row-local projection + window predicate over the
   source, delete flag projected, no window function.
4. `emit_succession_patch` — one transactional `StatementGroup`: the idempotent tombstone insert
   (anti-join on `(k, t)`), then the presented `MERGE` whose `USING` recomputes `LEAD`/`LAG` over
   the neighbour domain (presented ∪ ledger ∪ batch) restricted to the batch rows and their
   immediate neighbours, keyed on `(k, t)`. Dialect-keyed on `MaintenanceDialect` like
   `emit_keyed_fold`; DuckDB is the only branch this phase must render correctly — Spark and
   BigQuery take the recorded downgrade (outcome §Out of scope), so refuse rather than emit
   half-right text for them.
5. `emit_succession_ledger_rebuild_select` and `emit_succession_clock_tie_probe`.
6. `crates/smelt-state/src/ddl_duckdb.rs`: `generate_tombstone_table_ddl(qualified_name, key_cols
   with types, clock col with type)` + drop, doc-commented as bookkeeping DDL under the
   maintenance-plan-purity carve-out.
7. Write tests 1–14, red first, then the emitters until green.
8. Add the conformance cell (test 15) and extend the structural leg (test 16).
9. Check `.claude/large-file-baseline.txt` is untouched by the new files; do not `--update` it.

## Verification

- `cargo test -p smelt-logical --test succession_emit`
- `cargo test -p smelt-logical --test maintenance_plan_conformance`
- `cargo test -p smelt-state`
- `cargo test -p smelt-runtime --test statement_parity`
- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, workspace tests,
  `example_diagnostics`) — green at the start of this phase per the 3c summary, so any red is
  this phase's.

## Commit message

`feat(smelt-logical): succession-patch emitters, tombstone-ledger DDL, and the append-only SCD2 conformance cell`
