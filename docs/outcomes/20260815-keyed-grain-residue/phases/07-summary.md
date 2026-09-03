# Phase 7 summary — Make the non-DuckDB `Grade::Idempotent` ledger skip a recorded, visible fact

**Shipped:**
- `RunReporter::state_structure_unavailable(run_id, model, structure, dialect, consequence)` —
  defaulted no-op method on the trait (`crates/smelt-runtime/src/reporter.rs`).
- `ReporterEvent::StateStructureUnavailable` variant, `EventSink` push/replay arms
  (`crates/smelt-runtime/src/execute.rs`) — buffers the fact under `--jobs > 1` like every other
  reporter callback.
- `maintenance_driver.rs`'s `Grade::Idempotent` non-DuckDB else-branch now calls
  `retry.reporter.state_structure_unavailable(...)` instead of `tracing::warn!`; the run still
  succeeds (bookkeeping, not a correctness gate).
- `CliReporter::state_structure_unavailable` — a `tracing::warn!` naming model/structure/dialect,
  visible in ordinary `smelt run` output (`crates/smelt-cli/src/reporter.rs`).
- Spec: `docs/specs/incremental_shapes.md` §"The transactional frontier write (merge ledger)" now
  states the omission is reported, never silently dropped, and the additive grade still refuses;
  the matching Known Divergences bullet gained a one-clause update (no pre-judgment of phase 3).
- Docs-site: `docs-site/docs/reference/state.md` §"The reconciliation ledger" — one sentence on
  the reported skip.
- Tests: `idempotent_ledger_skip_on_non_duckdb_is_reported`,
  `idempotent_ledger_on_duckdb_reports_no_unavailability` (`maintenance_driver.rs`),
  `buffered_state_structure_unavailable_replays_to_reporter` (`execute.rs`).

**Decisions:**
- `smelt-ui`'s `BroadcastReporter` gets no override — confirmed it doesn't implement the sibling
  `model_retrying`/`check_result` events either (it translates into its own typed
  `RunProgressEvent` protocol rather than forwarding verbatim), so per the plan's condition the
  default (no-op) stands.
- Grepped the file's three remaining `tracing::warn!` sites: all are fingerprint-sidecar
  staleness detection / probe-fallback logging, not a state-structure write skip — out of scope,
  noted per plan task 6, nothing else needed changing.

**For the next planner:** nothing new discovered. Row 8 (close-out: `/smelt:validate
incremental_shapes` clean, all standing gates green) is next; phase 3 stays blocked pending a
human decision among its three candidate options.

**Gates:** `bash .claude/scripts/verify-phase.sh` (ALL GREEN), `cargo test -p smelt-runtime --lib
maintenance_driver` (28 passed), `cargo test -p smelt-runtime --test keyed_frontier_bookkeeping
--test statement_parity --test execute_parity` (33 passed), `cargo test -p smelt-cli --test
maintenance_conformance` (75 passed), `cargo check -p smelt-maintenance-testkit --features
spark,bigquery --all-targets` (clean).
