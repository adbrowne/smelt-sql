# Phase 2 plan — Frontier record for re-run-tolerant window-forward keyed models

## Objective

Make a `Grade::Idempotent` (re-run-tolerant) window-forward keyed model write a merge-ledger
record for every merged window, transactionally with the write, matching §"The transactional
frontier write (merge ledger)"'s unqualified "every **window-forward** keyed model maintains a
per-model frontier". Today only additive-graded cells create the ledger table, so a fully
idempotent model leaves `--auto` nothing to consult. Advances success criterion 2 (and is a
prerequisite for 4's `--auto` bookkeeping story).

## Spec delta

`docs/specs/incremental_shapes.md` §Known Divergences: delete the bullet **"Re-run-tolerant
keyed models do not yet write the frontier"**. The DuckDB-only ledger substrate stays recorded
by the existing "reconciliation ledger's fold is transactional on DuckDB only" bullet — extend
that bullet's wording to say the bookkeeping-graded record is likewise written on DuckDB only
(phase 3 closes it). No normative text changes: §"The transactional frontier write" already
states the target, and `state.md` §"`state.mode` and what each posture provides" already puts
correctness structures in every posture, so **no `state.mode` gate** is added here.

## Design (fixed, so the implementer does not re-decide)

- The bookkeeping insert must tolerate a re-merge: add
  `smelt_state::ddl_duckdb::generate_ledger_upsert_sql` — the same `INSERT` as
  `generate_ledger_insert_sql` plus `ON CONFLICT DO NOTHING`, so a recorded window re-merges as
  a no-op instead of tripping the never-fold-twice `PRIMARY KEY`. `smelt-state` stays the single
  owner of ledger DDL/DML (the bookkeeping carve-out in CLAUDE.md §maintenance-plan purity).
- Generalise the backend seam rather than growing a parallel one: add
  `Backend::execute_write_with_bookkeeping(&self, ensure_sqls: &[String],
  pre_write_sqls: &[String], write_group: &StatementGroup)` (default: each ensure, each
  pre-write, then `execute_statement_group`), and re-express
  `execute_conditional_write_and_record_observed_delta` as a defaulted thin delegation to it.
  Move DuckDB's existing transactional override onto the new method so exactly one transactional
  implementation exists. Pre-write ordering is preserved because the observed-delta record must
  read pre-write target state.
- Driver (`run_windowed_keyed_maintenance`, `Grade::Idempotent` arm): when
  `backend.dialect() == SqlDialect::DuckDB`, build `(ledger_ensure, ledger_upsert)` from the
  step's own partition value / range with `LEDGER_WHOLE_ROW_GROUP` and `rule.ledger_input()` —
  identical keying to the `Grade::Additive` arm — and pass them through
  `execute_write_with_bookkeeping` in **both** sub-branches (suppressed: appended after the
  observed-delta ensure/record; plain: on their own). Record the first (table-creating) step too
  — that window is merged state. On a non-DuckDB dialect skip the record with a `tracing::warn`
  naming the DuckDB-only ledger substrate; do **not** `bail!` (the additive arm's refusal is a
  correctness gate, this is bookkeeping, and bailing would regress working Spark keyed models).
- Snapshot-reconcile models keep no frontier — that path does not run this driver; assert it.

## Tests

- `crates/smelt-state/src/ddl_duckdb.rs::ledger_upsert_is_conflict_tolerant` — the generated
  upsert carries the same column list/values as the plain insert plus `ON CONFLICT DO NOTHING`.
- `crates/smelt-backend-duckdb/src/lib.rs::write_with_bookkeeping_rolls_back_on_write_failure` —
  a failing `write_group` leaves no bookkeeping row behind (the transactional override).
- `crates/smelt-backend-duckdb/src/lib.rs::write_with_bookkeeping_runs_pre_writes_before_the_write`
  — ordering guarantee the observed-delta record depends on, now asserted on the new seam.
- `crates/smelt-runtime/tests/keyed_frontier_bookkeeping.rs` (new, real DuckDB via
  `execute_project`):
  - `idempotent_keyed_model_records_every_merged_window` — a `MAX`-combiner keyed model over two
    day partitions leaves exactly two ledger rows keyed `(model, whole-row group, input,
    partition)`.
  - `re_running_a_recorded_window_is_a_no_op_not_a_refusal` — a second run over the same range
    succeeds, produces no `KeyedReprocessedWindow`, and leaves the row count unchanged.
  - `snapshot_reconcile_model_writes_no_frontier_record` — the ledger holds no row for a
    snapshot-reconcile keyed model.

## Tasks

1. Add `generate_ledger_upsert_sql` (+ unit test) to `crates/smelt-state/src/ddl_duckdb.rs`.
2. Add `Backend::execute_write_with_bookkeeping` to `crates/smelt-backend/src/lib.rs`; redefine
   `execute_conditional_write_and_record_observed_delta` as a delegation to it.
3. Move the DuckDB transactional override onto the new method
   (`crates/smelt-backend-duckdb/src/lib.rs`); keep the existing observed-delta tests green and
   add the two new ones.
4. Wire the `Grade::Idempotent` bookkeeping record into `run_windowed_keyed_maintenance`
   (`crates/smelt-runtime/src/maintenance_driver.rs`), both sub-branches, with the non-DuckDB
   `tracing::warn` skip; update the arm's doc comment (it currently claims "`Grade::Idempotent`
   cells skip the ledger entirely — no warehouse table is ever created for them").
5. Add `crates/smelt-runtime/tests/keyed_frontier_bookkeeping.rs`.
6. Apply the spec delta above.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test keyed_frontier_bookkeeping`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` (the group recorded
  by `execute_statement_group` must be byte-unchanged — the new seam wraps it, never rewrites it)
- `cargo test -p smelt-cli --test maintenance_conformance` (re-run tolerance under the new
  bookkeeping insert)
- `cargo test -p smelt-backend-duckdb`

## Commit message

`feat(incremental): re-run-tolerant keyed models record every merged window in the merge ledger`
