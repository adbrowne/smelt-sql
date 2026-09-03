# Phase 5 plan — Generative conformance pool: nullable payload, once-write NULL direction covered

## Objective

Make the generative conformance pool's row type carry a nullable payload (`GenRow::val:
Option<i64>`) and prove the once-write family's NULL-preservation obligation from a *generated*
schedule rather than a single hand-written case. Advances success criterion 5, and removes the
`incremental_shapes.md` Known Divergences bullet criterion 6 will check.

## Spec delta

`docs/specs/incremental_shapes.md` §Known Divergences — delete the bullet "**The generative
conformance pool cannot stage NULL payloads (Open Question)**" (currently ~line 1229). It states
the exact gap this phase closes; nothing replaces it, because the harness now covers the direction
the bullet says it cannot. No normative surface changes: this is a test-harness capability, so no
`docs-site/` page is affected.

## Tests

Red-green, in this order:

1. `smelt-maintenance-testkit` unit — `gen_row_null_payload_renders_sql_null`: `GenRow::val_sql()`
   yields `NULL` for `None` and the bare integer literal otherwise (the one rendering seam every
   insert/oracle site funnels through).
2. `smelt-maintenance-testkit` unit — `s_view_select_sql_types_a_leading_null_payload`: when the
   FIRST row of `STracker::s_view_select_sql`'s `UNION ALL` chain has a NULL payload, the leading
   branch emits a *typed* null (`CAST(NULL AS INTEGER) AS <val>`), so the union's column type is
   not the untyped NULL type (Spark and BigQuery both reject/mistype the untyped form).
3. `smelt-maintenance-testkit` unit — `arb_once_write_null_schedule_preserves_the_world_fact`: over
   a deterministic sample, every generated schedule carries at most ONE distinct non-NULL value per
   key (the once-write provenance proof's precondition, `incremental_shapes.md` §"The column-family
   catalogue"), and at least one sampled schedule delivers a NULL for a key strictly before its
   real value.
4. `smelt-cli --test maintenance_conformance` — `once_write_null_pool_upholds_end_state_equivalence`:
   a deterministic proptest sample over `arb_once_write_null_schedule()` crossed with all three
   once-write spellings (`OnceWrite`, `OnceWriteFallback`, `OnceWriteMultiCandidate`), each staged
   and driven through the real `execute_project` pipeline by `drive_keyed_and_assert`, asserting the
   `STracker` full-refresh oracle after every window. This is the test that replaces the
   hand-written case as the *proof*.
5. Existing `once_write_null_payload_then_value_upholds_equivalence` keeps passing, retained as a
   pinned minimal witness (see task 7).

## Tasks

1. `schedule_gen.rs`: change `GenRow::val` to `Option<i64>`; add `GenRow::new(d, id, val: i64)`,
   `GenRow::null(d, id)`, and `pub fn val_sql(&self) -> String` (the single `NULL`-or-literal
   renderer). Update the type's doc comment, which today asserts a non-nullable payload.
2. Thread the type change through `smelt-maintenance-testkit` construction and read sites:
   `schedule_gen.rs`, `recipe.rs`, `s_tracker.rs`, `feed.rs` (including the `Update` step's
   `existing.val = val`), `probes.rs`, `families/{gate,gate_keyed,gate_mixed,gate_composed,pinned,mod}.rs`.
   Existing literal payloads become `Some(..)`; behaviour is unchanged for them.
3. Route every SQL-rendering site through `val_sql()`: `insert_row_keyed_for`
   (`families/gate_keyed.rs`), the other family inserters, `STracker::materialize_rows`'s `INSERT`,
   and `s_view_select_sql` — the last with the typed-null leading branch from test 2.
4. Arrow/snapshot readers must read a NULL payload as `None` rather than `value(i)`:
   `read_source_snapshot` (DuckDB) and the Spark-side reader in `schedule_gen.rs` (~lines 530-610),
   plus `read_source_snapshot_via_backend`.
5. Update construction sites in the test crates: `crates/smelt-cli/tests/maintenance_conformance/`
   (`gate.rs`, `pinned.rs`, `probes.rs`, `contract_points.rs`) and the `_spark` / `_bigquery` twins.
6. Add `arb_once_write_null_schedule()` to `recipe.rs` beside `arb_keyed_schedule`: 2-3 one-day
   windows over a shared re-touched key plus fresh keys, where each key draws ONE non-NULL value
   and an optional prefix of NULL-payload deliveries in earlier windows — world-fact-preserving by
   construction, and the direction a total (fallback-carrying) projection would break. Keep
   `arb_payload_value()` and the general append-only pool non-NULL (see the outcome decision log).
7. Add test 4 in `crates/smelt-cli/tests/maintenance_conformance/gate.rs` beside
   `once_write_pool_upholds_end_state_equivalence`. Retitle
   `once_write_null_payload_then_value_upholds_equivalence` as a pinned minimal witness and rewrite
   its doc comment: its current justification ("`GenRow::val` is a non-nullable `i64` … making it
   nullable is a generator-wide change out of proportion") becomes false with task 1 and must not
   survive.
8. Sweep the testkit for other doc comments asserting a non-nullable payload
   (`rg -n 'non-nullable|nullable' crates/smelt-maintenance-testkit crates/smelt-cli/tests`) and
   correct them.
9. Apply the spec delta.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, full workspace tests,
  `example_diagnostics`).
- `cargo test -p smelt-maintenance-testkit` — the three new unit tests plus the existing suite.
- `cargo test -p smelt-cli --test maintenance_conformance` — the new pool test and every
  pre-existing keyed/once-write case still green.
- `cargo check -p smelt-cli --tests --features smelt-cli/spark` and
  `cargo check -p smelt-cli --tests` (BigQuery twin) — the gated twins must still COMPILE against
  the new row type even though their engines do not run here.
- `rg -n 'GenRow::val' docs/specs/` returns nothing (spec delta applied).

## Commit message

`test(conformance): nullable GenRow payload; generate the once-write NULL direction`
