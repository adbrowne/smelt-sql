# Phase 2 summary — Frontier record for re-run-tolerant window-forward keyed models

**Shipped:**
- `smelt_state::ddl_duckdb::generate_ledger_upsert_sql` — the same `INSERT` as `generate_ledger_insert_sql`
  plus `ON CONFLICT DO NOTHING`, with a unit test proving the column list/values match.
- `Backend::execute_write_with_bookkeeping(ensure_sqls, pre_write_sqls, write_group)` — a generalised
  seam (`crates/smelt-backend/src/lib.rs`) with a best-effort non-atomic default; DuckDB overrides it
  with a real transaction over `pre_write_sqls` + `write_group`, ensures run first outside the
  transaction. `execute_conditional_write_and_record_observed_delta` is now a thin delegation to it;
  the DuckDB observed-delta override was removed (folded into the new method), so exactly one
  transactional implementation exists.
- `run_windowed_keyed_maintenance`'s `Grade::Idempotent` arm now builds `(ledger_ensure, ledger_upsert)`
  every step, identically keyed to the `Additive` arm (`LEDGER_WHOLE_ROW_GROUP`/`rule.ledger_input()`),
  and passes it through `execute_write_with_bookkeeping` — appended after the observed-delta
  ensure/record in the suppressed sub-branch, standalone in the plain sub-branch (which also covers the
  first, table-creating step). On non-DuckDB, the record is skipped with a `tracing::warn`, not a `bail!`.
- New integration suite `crates/smelt-runtime/tests/keyed_frontier_bookkeeping.rs` (3 tests, real DuckDB
  via `execute_project`): every merged window (incl. the first) gets a ledger row; re-running an
  already-recorded window succeeds silently (no `KeyedReprocessedWindow`) and the row count doesn't grow;
  a snapshot-reconcile keyed model creates no ledger table at all.
- Spec delta applied: `docs/specs/incremental_shapes.md` §Known Divergences — deleted the "re-run-tolerant
  keyed models do not yet write the frontier" bullet; folded into the DuckDB-only ledger bullet (now
  covers both additive and idempotent records, and states the ledger table itself is DuckDB-only).

**Decisions:**
- Followed the plan's fixed design exactly: no `state.mode` gate on the bookkeeping write (`state.md`
  already places correctness structures in every posture); `ON CONFLICT DO NOTHING` rather than a new
  refusal path for idempotent re-merges.
- Updated the stale doc comment on `run_windowed_keyed_maintenance` and the stale module comment in
  `ddl_duckdb.rs` claiming idempotent cells never touch the ledger — both now describe the shared table.

**For the next planner:**
- `maintenance_driver::tests::sequences_create_then_merge_across_partitions_in_temporal_order` (a
  pre-existing unit test using `SumRule`'s default `Grade::Idempotent` grading against a DuckDB-dialect
  `RecordingBackend`) needed its call-count assertions updated from 3 to 9 calls — this is the expected,
  intentional behavior change this phase makes, not a regression.
- Phase 3 (transactional ledger fold on every shipped backend) is a direct continuation: the ledger
  substrate (both additive `INSERT` and idempotent upsert) is still DuckDB-only; a non-DuckDB
  window-forward keyed model still writes no frontier record. This phase's `tracing::warn` skip in the
  `Idempotent` arm is the same posture the `Additive` arm's `bail!` documents as a placeholder for phase 3.
- No new out-of-scope discoveries beyond what phase 1 already surfaced (the empty-`JoinContext`
  per-group-repair limitation, already tracked under the repair-family outcome).

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full workspace
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test keyed_frontier_bookkeeping` — 3/3 passed.
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — 33/33 + 4/4 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 74/74 passed.
- `cargo test -p smelt-backend-duckdb` — 32/32 (+ other test files) passed.
- `cargo test -p smelt-state` — 298/298 + supporting suites passed.
