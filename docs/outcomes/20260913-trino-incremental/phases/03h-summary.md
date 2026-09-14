# Phase 3h summary — the whole-row `MERGE` upsert family proven live on Trino

**Shipped:**
- `crates/smelt-cli/tests/trino_incremental_families.rs`: `stage_keyed_fold_project` (parameterised MIN/SUM combiner over a clocked, append-only `events` source) plus three live tests — `whole_row_merge_upsert_matches_full_refresh_on_trino` (idempotent, two disjoint windows, multiset-equal to a `--full-refresh` oracle), `whole_row_merge_upsert_writes_through_the_merge_route_on_trino` (`--json` shows `KeyedFold` with no `state_downgrade`), `additive_keyed_fold_downgrades_and_still_matches_full_refresh_on_trino` (`SUM`, downgraded and explain-visible, oracle-equal after each of two windows).
- `crates/smelt-runtime/tests/statement_parity/trino.rs` (new): `keyed_fold_parity_on_trino` — a generalized `RecordingBackend` (`inner: Box<dyn Backend>`) wraps a live `TrinoBackend`; the executed `MERGE` group is asserted byte-identical to a direct `emit_keyed_fold_suppressed` call.
- `crates/smelt-runtime/tests/common/mod.rs` (new): the crate's own `trino_env`/`trino_schema`/`trino_backend`/`drop_trino_schema` helpers, mirroring `smelt-cli`'s, loaded into the `statement_parity` binary via `#[path = "../common/mod.rs"] mod common;` in `main.rs` — required so `trino_ci_wiring.rs`'s anti-duplicate-schema-helper gate (which scans each live-gated binary's own text) doesn't flag a private copy.
- `smelt-runtime`'s `RecordingBackend`/`RecordingBackendFactory` (`statement_parity/main.rs`) generalized to `Box<dyn Backend>`; every direct `RecordingBackend::new(inner)` call site across the suite updated to `Box::new(inner)`. The factory's own DuckDB construction is untouched (kept dispatching off `self.db_path` directly, not `smelt_backends::create_backend`, after that generalization attempt broke fixtures with no `database:` field in their `smelt.yml`).
- `crates/smelt-runtime/Cargo.toml`: `smelt-backend-trino` added as a dev-dependency (no cycle — verified via `cargo tree -p smelt-backend-trino -i smelt-runtime`).
- `trino_ci_wiring.rs`'s `live_gated_census()` widened to `["smelt-cli", "smelt-backend-trino", "smelt-runtime"]`; `.github/workflows/compat.yml`'s `trino-integration` job gained a `Run smelt-runtime statement_parity Trino leg` step.
- `trino_incremental_families.rs`'s module doc updated: gaps 4/5 marked closed (3f/3g), phase 3h's three tests documented.

**Decisions:**
- 2026-09-15: Kept `RecordingBackendFactory`'s DuckDB path exactly as before rather than routing it through `smelt_backends::create_backend` — that shared factory function requires a `database` field even when a `database_override` is supplied, which several existing fixtures' `smelt.yml` omit (relying entirely on the test's own `db_path`). Generalizing to `Box<dyn Backend>` only needed the *type*, not a shared construction path; a separate `TrinoRecordingBackendFactory` in `trino.rs` builds the `TrinoBackend` directly.
- 2026-09-15: The `device_agg` fixture (both `smelt-cli`'s and `smelt-runtime`'s copies) declares `maintenance.scan_bounds.per_source.events.allow_full_scan: true` even for the idempotent (`MIN`) leg — measured live that a `MaintenanceScanUnbounded` diagnostic refuses the build without it when the driving relation is a declared external source (`sources/events.yml`), unlike a plain first-class model with inline `timeseries:` frontmatter (the shape `region_and_keyed_fold.rs`'s DuckDB fixture uses, which needs no such declaration). Matches `keyed_fold_state_downgrade_execution.rs`'s existing DuckDB fixture, which already carries this for the same reason.

**For the next planner:**
- Phase 4 is next: the emulated delete-and-insert window family.
- The 3g summary's "untouched watch item" (`realises_merge_ledger(dialect)`'s DuckDB + `warehouse_tables: none` gap) remains unexercised — Trino's own `realises_merge_ledger` returns `false` regardless, so this phase's live leg cannot surface it either; still only a hypothetical.
- `statement_parity`'s Trino leg proves only the idempotent (`MIN`) `MERGE` shape's byte-identity; the additive downgrade route's *statements* (the whole-target rebuild) are proven live only via `trino_incremental_families.rs`'s result-equality assertion, not a `statement_parity`-style byte-identity check against `emit_create_table_as`/`emit_full_refresh` — worth adding if a future phase touches that emission path.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck, full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` (no live tier) — 46 passed, Trino leg skips cleanly.
- `cargo test -p smelt-cli --test trino_ci_wiring` — 8 passed (including the widened census's anti-duplicate-helper and no-skip gates).
- Live tier (`bash scripts/trino-up.sh` / `source scripts/trino-env.sh` / `bash scripts/trino-down.sh`):
  `cargo test -p smelt-runtime --test statement_parity -- --test-threads=1` — 42 passed, including `trino::keyed_fold_parity_on_trino`.
  `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1` — 7 passed, including all three new tests.
  `cargo test -p smelt-cli --test trino_ci_wiring` — 8 passed (re-run with the tier up).
