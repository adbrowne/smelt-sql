# Phase 32b summary — posture-derived key departure, runtime half

## Shipped

- `emit_departed_key_delete` (`crates/smelt-logical/src/maintenance/emit.rs`): the default
  point's anti-join `DELETE`, null-safe key equality per dialect (`IS NOT DISTINCT FROM`
  DuckDB/BigQuery, `<=>` Spark), multi-column key.
- `reconcile_disposition` + `DepartedKeyDisposition` (`crates/smelt-logical/src/contract/
  retain_departed.rs`): the pure write-path seam resolving `Option<&RetainDeparted>` to
  `Delete` or `Retain { tombstone }`.
- `execute_snapshot_reconcile` (`crates/smelt-runtime/src/cumulative.rs`) now assembles the
  merge and (when undeclared) the delete into one `transactional: true` `StatementGroup`
  executed via `execute_statement_group`; when `retain_departed` is declared, it instead
  dispatches `emit_departed_key_probe` pre-write and fails loud on a non-zero unmarked-
  tombstone count (self-contained `anyhow::ensure!`, not yet the persisted `ProbeRecord`
  ledger — see below).
- Spec deltas: removed the "runtime half is unimplemented" bullet from
  `incremental_models.md` §Known Divergences and the "still retained" bullet from
  `incremental_shapes.md` §Known Divergences; refreshed the stale doc comments in
  `cumulative.rs` and `contract/mod.rs`.
- Tests: `emit_departed_key_delete_shape`, `reconcile_disposition_ladder` (unit),
  `snapshot_reconcile_deletes_departed_key`, `snapshot_reconcile_retains_departed_key_when_declared`,
  `retain_departed_probe_is_dispatched_pre_write` (new `tests/departed_key_reconcile.rs`,
  live DuckDB), `snapshot_reconcile_delete_leg_parity` (extends `tests/statement_parity.rs`).
  Repaired `crates/smelt-cli/tests/maintenance_conformance/gate.rs`'s
  `snapshot_reconcile_plain_overwrite_settles_with_retained_departed_keys` (renamed
  `..._and_deletes_departed_keys`), which had pinned the old silent-retention default.

## Decisions

- The probe dispatch (declared-point half) is implemented directly inside
  `execute_snapshot_reconcile` rather than threaded through `smelt-runtime`'s
  `ProbeRecord`/`ModelRunRecord.probes` ledger the `frozen_horizon`/`deferral` probes use.
  That ledger is populated by a different code path in `execute.rs` (the batched
  window-forward per-cell maintenance-plan loop, ~line 3579) that the snapshot-reconcile
  branch (~line 2407) never joins — plumbing a `ModelRunRecord` through there is a
  materially bigger, separable change. The probe still runs pre-write and still fails the
  run loud on a genuine violation; it just isn't recorded in the run manifest yet.
- `execute_snapshot_reconcile`'s reconcile branch builds the `StatementGroup` inline rather
  than extending `build_cumulative_merge_sql` (which is shared with the window-forward path,
  where departure is not observable per `incremental_shapes.md` §"Departed keys and
  deletion") — keeps that shared helper's signature and every existing caller untouched.

## For the next planner

- **Follow-up, in scope for a future phase**: wire `retain_departed`'s probe outcome into
  the persisted `ProbeRecord`/`ModelRunRecord.probes` ledger for the snapshot-reconcile path,
  matching `frozen_horizon`/`deferral`'s precedent, so `smelt explain`/run reports surface it
  the same way. Not required by this phase's spec delta (the spec bullets removed were about
  the *default write behavior*, not manifest surfacing), but worth a line item.
- The plan's Verification section named `cargo test -p smelt-cli --test example_web_analytics`;
  that test target lives in `smelt-datagen`, not `smelt-cli` (crate drift since the plan was
  written). Ran both `smelt-datagen --test example_web_analytics` and
  `smelt-cli --test web_analytics_incremental_classification` instead — both green, no
  fixture depended on the old silent-retention default.
- No other fixture besides the one repaired in `gate.rs` assumed retention; the
  `examples/web_analytics` and `examples/timeseries` suites were unaffected (no
  `mutable_snapshot` keyed model in those examples currently has a departing key in its
  fixture data).

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test statement_parity` — 33 passed.
- `cargo test -p smelt-runtime --test departed_key_reconcile` — 3 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 74 passed.
- `cargo test -p smelt-cli --test web_analytics_incremental_classification` and
  `cargo test -p smelt-datagen --test example_web_analytics` — both green (substituting for
  the plan's stale `smelt-cli --test example_web_analytics` target name).
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed.
