# Phase 9 summary — backend-aware downgrade visibility

## Shipped

- `smelt_db::maintenance_plan_report` now takes `dialect_name: &str` and resolves
  `StateAvailability` via `state_availability_for(dialect_name)` instead of the DuckDB-optimistic
  `StateAvailability::all()` (`crates/smelt-db/src/lib.rs`). This is the single function that
  feeds `smelt explain`'s report and JSON — it already called `derive_model_maintenance_plan_with_edges`
  unconditionally (edge-aware for every model, empty edge list when none), so no separate
  "graph path" fix was needed.
- `crates/smelt-cli/src/commands/explain.rs`: `explain_maintenance_plan` now resolves the
  model's real target backend (`config.get_target` → `BackendType` → `"duckdb"`/`"spark"`)
  *before* deriving the plan, and passes it through. Verified live: `smelt explain
  dag_kchain_b` against a Spark-only-targeted fixture now prints `state downgrade: DeleteInsert
  (ideal: DeleteInsert, missing FrontierRecord) — no engine-resident frontier builder for this
  backend; ...` (previously printed nothing — every path passed `all()`).
- `--json`: `ExplainMaintenanceJson` gained a `state_downgrades` array
  (`crates/smelt-cli/src/explain.rs`); `build_maintenance_plan_json` takes
  `state_downgrades: &[StateDowngrade]` and renders `{cell_group, trigger, resolved_technique,
  ideal_technique, missing_structure, why}` per entry. Documented in
  `docs/specs/cli.md` §"`smelt explain --json` output schema" section (spec-first, added before
  the code per the plan).
- Audited every remaining `StateAvailability::all()` site (plan task 5):
  - `crates/smelt-runtime/src/maintenance_driver.rs`'s four execution-time resolvers
    (`resolve_incremental_strategy`, `resolve_live_column_scoped_cell`,
    `resolve_live_in_place_update_cell`, `resolve_live_membership_recompute_cell`) now take a
    `dialect: SqlDialect` param, threaded from `execute.rs`'s already-in-scope `backend.dialect()`
    at all 5 call sites. For duckdb-only runs (the only backend in the ungated test suite) this
    is a no-op; it only changes behavior for a live Spark run.
  - `resolve_live_delta_restriction_facts` (maintenance_driver.rs) and
    `propagation.rs`'s edge walk are left at `all()` with a one-line comment: the former only
    reads row-identity/closure facts no availability downgrade touches; the latter already had
    a correct pre-existing rationale (dirt-interval facts want the un-downgraded ideal plan).
- Fixed a related latent bug the phase's own golden-fixture test exposed: `default_target`
  resolution in three call sites (`explain.rs` ×2, `smelt-ui/build.rs`) used
  `config.targets.keys().next()` over a `HashMap` — hash-seed-dependent, so a multi-target
  project (`examples/timeseries`: `dev`/`spark`) picked a different "default" target
  nondeterministically per process. Changed to `.keys().min()` for determinism.
- Threaded the real dialect (or a documented `"duckdb"` literal where the caller is
  provably duckdb-only, e.g. `smelt bakeoff`) through every other call site of
  `maintenance_plan_report`: `smelt-ui/src/build.rs`, `smelt-cli/src/bakeoff.rs`,
  `smelt-maintenance-testkit`'s `verdict.rs`/`dag.rs` (new `dialect_name_for_config` helper in
  the testkit crate's `lib.rs`, config-derived), plus ~15 test files including the
  `maintenance_conformance_spark` suite (now genuinely tests Spark availability instead of an
  accidental `all()`).
- New tests in `crates/smelt-cli/tests/explain_maintenance.rs`:
  `spark_target_model_explains_state_downgrade`, `duckdb_target_model_explains_no_state_downgrade`,
  `explain_json_carries_state_downgrades`, `explain_graph_path_resolves_real_availability` — all
  stage the existing `keyed_chain_dag` fixture (`dag_kchain_a` → `dag_kchain_b`, a `KeyedFold`)
  under a Spark-vs-DuckDB `smelt.yml`. Plus
  `state_availability_for_spark_withholds_ledger_and_frontier` in `smelt-db`'s maintenance unit
  tests — this one was already green pre-fix (the resolver itself was always correct; every
  *caller* was the bug), noted rather than silently kept.

## Decisions

- Reused `keyed_chain_dag`'s DuckDB-staged fixture and overwrote `smelt.yml` in place with
  `render_smelt_yml_for(SparkDelta, ...)` rather than `stage_dag_for_target` — the latter opens a
  live Spark connection to create physical tables, which `smelt explain` never needs (it's fully
  offline) and which isn't available in this sandbox.
- `dialect.name()` (`"DuckDb"`/`"Spark SQL"`/`"PostgreSQL"`) is safe to lowercase-feed straight
  into `state_availability_for` without a translation table: that function only special-cases
  `"duckdb"`, everything else (including `"spark sql"`) falls into its `_ => none()` arm.
- Did not add a Spark ledger/frontier builder or otherwise change `state_availability_for`
  itself — out of scope (`docs/specs/state.md` "Out of scope").

## For the next planner

- The `maintenance_conformance_spark` test edits (passing `"spark"` instead of `all()`) are
  **untested here** — that suite needs a live Delta-enabled Spark Connect server
  (`scripts/spark-up.sh`), which this sandbox doesn't have. They compile cleanly under
  `--features smelt-cli/spark` and the change is semantically correct (these tests exist
  specifically to prove Spark-target behavior), but row 11's close-out sweep (or the next Spark
  parity run) should actually execute them once and confirm no fallout.
- Two `StateAvailability::all()` sites remain by design (`resolve_live_delta_restriction_facts`,
  `propagation.rs`'s edge walk) — each has an inline comment now explaining why. Row 10's docs
  sweep should not describe these as "not yet backend-aware" residue; they're intentional.
- Row 10 (docs-site + `/smelt:validate state` + Known Divergences narrowing) can now truthfully
  say `smelt explain` is backend-aware — the "Not (yet) backend-aware at this call site" comment
  this phase removed from `smelt-db/src/lib.rs` was the last one blocking that claim for the
  `smelt explain` path specifically.
- Found and fixed a genuine pre-existing nondeterminism bug (`HashMap::keys().next()` for
  default-target selection) as a side effect of this phase's own test flaking — worth a
  `rg 'targets.keys().next()'` sweep elsewhere in the codebase if any other latent copies exist
  (none found beyond the three fixed here).

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full
  `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model --test explain` — 55
  passed.
- `cargo test -p smelt-cli --test example_diagnostics` and
  `cargo test -p smelt-lsp --test example_workspaces` — both green (119/1 ignored, 34/34).
- `cargo test -p smelt-db --test integration` — 363 passed (includes the new
  `state_availability_for_spark_withholds_ledger_and_frontier` pin).
- `cargo check --workspace --tests` and `cargo check -p smelt-cli --tests --features
  smelt-cli/spark` — both clean (the spark-gated conformance tests compile; not executed, no
  Spark server available here).
- Manual: `smelt explain dag_kchain_b` against a live-staged Spark-only fixture prints
  `state downgrade: DeleteInsert (ideal: DeleteInsert, missing FrontierRecord) — no
  engine-resident frontier builder for this backend; ...` — pasted above under Shipped.
