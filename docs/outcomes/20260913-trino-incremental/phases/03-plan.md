# Phase 3 plan — `MaintenanceDialect::Trino`, and the append + whole-row-`MERGE` families executing

## Objective

Land `maintenance_dialect(SqlDialect::Trino)` — today an `Err`, and therefore the gate in front of
*every* maintenance family — and make the two ledger-free families it unblocks run end-to-end
through `execute_project` against the live tier: insert-only append, and the whole-row `MERGE`
upsert (the snapshot-reconcile keyed route, idempotent by key). Advances criteria 2 and 5's
executed-vs-emitted half; criterion 5's structural half stays phase 7's.

## Spec delta (made first, by the implement step)

`docs/specs/multi_backend.md`:

- §Known Divergences — **delete** the "No maintenance dialect on Trino" entry (it becomes false in
  this commit); the §"Incremental & schema evolution per backend" landing-state table is now the
  live statement of what runs.
- §"Incremental & schema evolution per backend" — the clause "`frozen_horizon`'s late-arrival
  verification probe is simply skipped with a run-time warning where Trino has no
  `MaintenanceDialect` to render it in" no longer describes Trino. Restate it: with a
  `MaintenanceDialect` landed, the probe renders on Trino like any other dialect; the skip-with-
  warning route in `contract_probes.rs` remains for a dialect that has none. (Whether the probe is
  *proved* on Trino stays criterion 9 / phase 9.)
- §"Whole-row MERGE" needs no edit — phase 2 already states Trino's column-by-column both-arms
  form, which this phase implements verbatim.

`crates/smelt-cli/tests/trino_incremental_spec_freshness.rs` and
`trino_emission_spec_freshness.rs` assert against that prose and must be re-pointed, not relaxed.

## Tests (red first)

1. `smelt-backend` unit — rename/replace
   `maintenance_dialect_is_ok_for_the_three_implemented_dialects_and_err_for_trino`: all four
   dialects now resolve, each to its own variant.
2. `smelt-logical` `merge.rs` unit — `whole_row_update_set` for Trino renders `c = alias.c` per
   output column (never `*`), and the not-matched arm renders
   `INSERT (c1, c2) VALUES (alias.c1, alias.c2)` (never `INSERT *` / `INSERT ROW` / `INSERT ROW`'s
   shorthand), matching phase 1's measured grammar.
3. `smelt-logical` `merge.rs` unit — `null_safe_eq` on Trino is `IS NOT DISTINCT FROM`
   (`multi_backend.md` §"Null-safe equality" already claims it).
4. `smelt-backend` unit — `require_merge_columns` refuses an empty column list on Trino with the
   same fail-loud error shape BigQuery gets (Trino has no star form either, so an empty list would
   emit a matched arm that assigns nothing).
5. `smelt-logical` unit — `supported_succession_dialect(Trino)` is `Err`, per the landing table's
   `SuccessionPatch` → downgrade row.
6. `crates/smelt-backend-trino/tests/backend_live.rs` (live) — `insert_into_from_query` appends the
   query's rows to an existing Iceberg table and leaves prior rows intact.
7. **New** `crates/smelt-cli/tests/trino_incremental_families.rs` (live, `trino_env()`-gated) — two
   `execute_project`-driven legs over a real Trino target:
   - append: an incremental model over an append-only source, run twice with disjoint windows, ends
     multiset-equal to a full refresh of the union;
   - whole-row upsert: a `grain: key` model over a `mutable_snapshot` source, run twice with the
     second run mutating one key's value, ends multiset-equal to a full refresh of the new source.
8. **New** `crates/smelt-runtime/tests/statement_parity/trino.rs` (live) — the executed-vs-emitted
   leg for both families: a recording wrapper over `TrinoBackend` captures the statements a real
   `execute_project` run sends, asserted byte-identical to direct `emit_keyed_fold` /
   `emit_departed_key_delete` (and the append path's insert) calls with the batch's own inputs —
   the same shape as `statement_parity/structural_and_ledger.rs::snapshot_reconcile_delete_leg_parity`.
9. `crates/smelt-cli/tests/trino_ci_wiring.rs` — the `trino-integration` job runs the two new
   binaries and still fails on a skipped leg.

## Tasks

1. Make the spec delta above.
2. Add `MaintenanceDialect::Trino` to `crates/smelt-logical/src/maintenance/emit/types.rs` and
   return it from `smelt_backend::maintenance_dialect`.
3. Fill every arm the new variant forces (`merge.rs`, `hash.rs`, `partition_bucket.rs`,
   `fingerprint.rs`, `bootstrap.rs`, `probes.rs`, `succession/mod.rs`,
   `smelt-runtime/src/maintenance_driver/sidecar.rs`). Discipline, stated per arm in a doc comment:
   an arm this phase *executes* (merge, null-safe equality) is measured against the tier; an arm a
   later phase executes carries the Trino spelling **and names the phase that will prove it**; an
   arm whose family is refused/downgraded on Trino returns the refusal where the site returns a
   `Result` (succession). No arm silently reuses another dialect's spelling without a note.
4. Change `whole_row_insert_arm` to take the column list (affects all dialects' call sites; DuckDB/
   Spark keep `INSERT *`, BigQuery keeps `INSERT ROW`) and extend `require_merge_columns` to Trino.
5. Implement `insert_into_from_query` on `TrinoBackend` (`INSERT INTO <catalog.schema.table>
   <select>`, the DuckDB/Spark precedent). Leave `delete_partitions` / `insert_overwrite` erroring —
   phase 4's.
6. Re-point the call sites and tests the landed `Ok(...)` flips: `staged_relation_atomicity.rs`,
   `trino_contract_points.rs`, `trino_explain_downgrade.rs`, `trino_posture_plan_invariance.rs`,
   `contract_probes.rs`'s skip route (keep the route, drop Trino from it).
7. Add `smelt-backend-trino` as a `smelt-runtime` dev-dependency; add `statement_parity/trino.rs`
   and the live family test; wire both into `compat.yml`'s `trino-integration` job with the same
   never-skip-green grep the existing steps use.

## Verification

- `bash scripts/trino-up.sh && source scripts/trino-env.sh` — **required**; if the coordinator
  cannot be reached, emit `<<PHASE_BLOCKED>>` rather than letting a live leg skip green.
- `cargo test -p smelt-backend-trino --quiet` (live legs must not print "skipping")
- `cargo test -p smelt-cli --test trino_incremental_families --test trino_incremental_spec_freshness
  --test trino_emission_spec_freshness --test trino_explain_downgrade --test trino_ci_wiring
  --test trino_posture_plan_invariance --quiet`
- `cargo test -p smelt-runtime --test statement_parity --quiet`
- `cargo test -p smelt-logical --quiet` and `cargo test -p smelt-backend --quiet`
- `bash .claude/scripts/verify-phase.sh`; `bash .claude/scripts/large-file-check.sh`
- `bash scripts/trino-down.sh`

## Commit message

`feat(trino): land MaintenanceDialect::Trino and run the append and whole-row-MERGE families`
