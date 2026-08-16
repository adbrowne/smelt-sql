# Phase 9 summary — Surface the definition-change refusal ahead of a run

**Shipped:**
- `ProjectInput` Salsa input gains `deployed_columns: Vec<(String, Vec<String>)>`
  (`crates/smelt-db/src/lib.rs`), the only mutation point `Database::set_project_deployed_columns`.
- `workspace_ingest::read_deployed_columns(project_root, target, mode)` (new, `smelt-db`) reads
  every model's deployed column names from `.smelt/targets/<target>/schemas/*.json` via
  `smelt_state::file_store::FileStore`; called from `ingest_loaded_workspace` and from the CLI's
  `init_db`/`build_execute_salsa_db` (target-override re-population) and the LSP's `initialize`.
  `smelt-db` gains `smelt-state` as a production dependency.
- `maintenance_plan` (Salsa query) and `maintenance_plan_report` (`smelt explain`) now read real
  deployed columns instead of a hardcoded `&[]`, so `MaintenanceSkeletonChanged` fires ahead of a
  run for both the LSP and `smelt explain`.
- LSP: `derive_watch_globs` adds a `.smelt/targets/*/schemas/*.json` watcher;
  `did_change_watched_files` re-reads deployed columns and republishes diagnostics on snapshot
  change, with no editor restart needed.
- Tests: 3 new `smelt-db` integration tests (`maintenance_diagnostics.rs`), 1 new `workspace_ingest`
  unit test, 1 new `smelt-cli` explain integration test, 2 new `smelt-lsp` integration tests (real
  `Backend` over duplex streams, new file `definition_delta_diagnostics.rs`).
- Spec deltas: `definition_deltas.md` §Detection gains the ahead-of-run paragraph, its Known
  Divergences bullet removed; `model_properties.md`'s matching bullet removed;
  `diagnostics.md`/`incremental_models.md` reachability prose updated.

**Decisions:**
- **Double-derivation to avoid a false-positive build regression.** Threading the real deployed
  snapshot straight into the primary maintenance-plan derivation surfaced `MaintenanceScanUnbounded`
  (and would surface other admission refusals) for ordinary nullable-column additions, because
  `smelt-logical`'s `ColumnAdded`/`InPlaceUpdate` technique assumes historical rows must be
  backfilled (needs an unbounded source scan) — but `smelt build`/`smelt run` never takes that
  route for ordinary schema evolution; they use `schema_evolution.rs`'s simpler ALTER-with-
  NULL-default route (phase 7), which needs no backfill and no scan bound. Caught by the existing
  `schema_evolution_incremental::add_column_on_incremental_model_alters_in_place` e2e test.
  Fix: `maintenance_plan_diagnostics` and `maintenance_plan_report` now derive the plan **twice**
  — a primary derivation with `&[]` (byte-identical to pre-phase-9 behaviour for every refusal
  kind), and a secondary derivation with the real snapshot consulted **only** to extract a
  `Refusal::SkeletonColumnAdded`, merged into the primary result. No `smelt-logical` change; no
  change to `schema_evolution.rs` semantics. `Refusal::ScanUnbounded` carries no `trigger` field,
  so a single-derivation filter wasn't possible without a cross-layer change — out of scope here.
- Effective target for deployed-columns lookup mirrors `set_active_target`'s own resolution
  (`smelt.yml` `target:`, default `"dev"`); CLI commands that override the target after `init_db`
  (`run`/`build`/`rebuild`) re-populate via `set_project_deployed_columns` for the real target.
  `smelt explain` uses `init_db`'s own config-file-default population (no override), matching
  existing precedent for that command.
- `hardening-baseline.txt`'s `smelt-db unwrap` count moved 16→17: one new `RwLock::read().unwrap()`
  in `set_project_deployed_columns`, same infallible-poisoning rationale as every sibling
  `set_project_*` method in the same file.

**For the next planner:**
- The double-derivation fix is scoped and safe but leaves a real architectural question open:
  should `smelt-logical`'s `Refusal::ScanUnbounded`/other admission refusals carry enough
  provenance (e.g. a `trigger` field) to distinguish "caused by a ColumnAdded backfill" from
  "caused by an ordinary NewData/UpstreamMutation trigger" in one derivation pass? That would let
  a single call replace the double-derivation. Not needed for this outcome's criteria, but worth a
  follow-up if more definition-change diagnostics need this reconciliation.
- `derive_column_added`'s "must backfill historical rows via an unbounded source scan" technique
  and `schema_evolution.rs`'s "just NULL old rows" route are two independently-evolved, not
  reconciled mechanisms for "add a nullable column". `smelt migrate`'s backfill catalogue is the
  thorough one (computes real historical values); ordinary runs take the cheap one. This split is
  now load-bearing (the double-derivation depends on it) — a future unification effort should read
  this phase's decision log entry first.
- Not touched: phases 10 (docs-site migration guide) and 11 (validate + close-out) remain pending.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy zero-warnings, full `cargo test`,
  example_diagnostics).
- `cargo test -p smelt-db --test maintenance_diagnostics --quiet` — 11 passed.
- `cargo test -p smelt-lsp --test example_workspaces --quiet` — 34 passed.
- `cargo test -p smelt-lsp --test definition_delta_diagnostics --quiet` — 2 passed (new).
- `cargo test -p smelt-cli --test explain_maintenance --quiet` — 30 passed.
- `cargo test -p smelt-runtime --test execute_parity --quiet` — 4 passed.
- `cargo test -p smelt-cli --test e2e schema_evolution_incremental --quiet` — 1 passed (the
  regression this phase's fix addresses).
