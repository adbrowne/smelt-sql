# Phase 7 summary — structural no-authoring leg for Trino, and the plan-derivation census

**Shipped:**
- `no_maintenance_statement_authoring_outside_the_emitter`'s scanned-crate list hoisted to
  `SCANNED_CRATES` (`crates/smelt-runtime/tests/statement_parity/structural_and_ledger.rs`), with
  `smelt-backend-trino` and `smelt-backend-bigquery` added. Trino measured clean (no allowlist
  entry needed); BigQuery needed three entries in the pre-existing `Backend::delete_partitions`/
  `create_table_as` class (`sql.rs:32`, `sql.rs:108`, and a `tracing::debug!` log line in `lib.rs`
  that echoes the same format-string shape).
- Three new tests proving criterion 5's second half:
  `trino_backend_is_in_the_no_authoring_scan_scope`, `bigquery_backend_is_in_the_no_authoring_scan_scope`,
  `a_trino_spelled_merge_in_a_backend_file_is_flagged` (synthetic double-quoted-`MERGE` fixture).
- New `crates/smelt-runtime/tests/statement_parity/plan_derivation_census.rs` (350 lines, declared
  from `main.rs`), landing criterion 6 in full: `maintenance_plan_is_derived_in_exactly_one_production_site`
  (every `derive_maintenance_plan`/`append_model_edge_cells`/`derive_triggers` call site outside
  `smelt-logical/src/maintenance/derive/` is exactly `smelt-db/src/queries/maintenance/plan.rs`),
  `census_flags_a_second_derivation_site` (scoping proof), `plan_consumers_hold_no_per_dialect_branch`
  (every `SqlDialect::`/`MaintenanceDialect::`/`BackendType::` hit across `smelt-db`'s maintenance
  queries/refs, `smelt-planner`, and `smelt-runtime`'s execute driver matches a named allowlist
  entry), `no_consumer_dialect_allowlist_entry_names_trino`.
- Spec delta: `CLAUDE.md`'s maintenance-plan-purity bullet and `docs/specs/architecture.md` §"Constraints
  & Invariants" item 12 (both the inline bullet and the "Known Divergences" entry) now state both
  halves — plan-derivation and statement-emission — are CI-gated, not conventional.

**Decisions:**
- The plan-consumer dialect-branch scan measured a 20th hit the plan's enumerated list didn't
  anticipate: `execute/project/mod.rs:69`, `refuse_databricks_cross_edges`'s
  `BackendType::Databricks` check (no host-visible warehouse path for `read_parquet()`).
  Pre-existing, unrelated to Trino — allowlisted with a reason noting it was measured here rather
  than in the phase plan. Appended to outcome.md's decision log.
- `no_consumer_dialect_allowlist_entry_names_trino` checks allowlist *metadata* (substring +
  reason text), not the underlying source lines — the underlying `targets.rs`/`write_pin.rs` code
  legitimately mentions `SqlDialect::Trino` as a selection-map arm; the criterion is that no
  allowlist *entry* was added *because of* Trino, which none were (all five predate this outcome).

**For the next planner:**
- Both halves of criterion 6/12 now have a standing gate; nothing deferred from this phase's own
  scope. Phase 8 (generative Trino conformance gate) and 9 (contract lattice) are next in the
  table, both `pending`.

**Gates:**
- `cargo test -p smelt-runtime --test statement_parity` — 54 passed, 0 failed (offline legs only;
  live-tier legs unchanged by this phase, tier not brought up).
- `cargo test -p smelt-runtime --test execute_parity` — 4 passed.
- `cargo clippy -p smelt-runtime --tests --quiet` — clean.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full workspace `cargo test`, `example_diagnostics`).
