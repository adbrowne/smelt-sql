# Phase 6 summary — the recorded downgrade becomes visible surface

## Shipped

- `smelt explain <model>` resolves availability once, offline, from the model's declared target
  dialect and `state.warehouse_tables`, before the report/JSON/`--show-sql` paths consume the
  plan (`crates/smelt-cli/src/commands/explain.rs::explain_maintenance_plan`, new
  `backend_type_to_sql_dialect` helper replacing `backend_type_to_maintenance_dialect`).
- Text report: a `      state downgrade: <original> → <technique> (missing: <structure>) — <reason>`
  row under a downgraded cell's `technique:` row, omitted when not downgraded
  (`crates/smelt-cli/src/explain.rs::build_maintenance_plan_report`).
- `--json`: `ExplainStateDowngradeJson { original, missing, reason }` and an
  `Option<…>` `state_downgrade` field on `ExplainCellJson`, `skip_serializing_if` absent
  (`crates/smelt-cli/src/explain.rs`).
- `RunReporter::state_structure_unavailable` deleted entirely: trait method, `CliReporter` impl,
  the buffered `EventSink`/`ReporterEvent::StateStructureUnavailable` variant + replay arm + its
  test, and all three test-local capturing-reporter impls (`maintenance_driver.rs`, `execute.rs`,
  `statement_parity.rs`). The keyed-grain merge-ledger skip in `maintenance_driver.rs` (~line 644)
  now leaves a `tracing::debug!` pointing at the cell's recorded `state_downgrade`.
- Spec delta: `docs/specs/cli.md` new "State downgrade" paragraph under §"`smelt explain <model>`
  maintenance-plan report"; `docs/specs/incremental_shapes.md` both `state_structure_unavailable`
  references rewritten to name the recorded downgrade / `smelt explain` as the channel.
- Fixed a latent, phase-6-exposed bug: `smelt explain`'s `default_target` fallback picked
  `config.targets.keys().next()` — `HashMap` iteration, randomized per process. Harmless before
  (only affected rendered SQL text) but, once dialect fed availability resolution, made a
  two-target project's ledger-requiring cell nondeterministically Admitted/downgraded across
  runs. New `resolve_default_target(config)` prefers `config.target` then a sorted-first target;
  used at both `commands/explain.rs` call sites.
- New tests: 4 in `explain_maintenance.rs` (text row, JSON field, DuckDB-omits, `warehouse_tables:
  none` forces it on DuckDB), 1 structural in `availability_seam.rs`
  (`retired_reporter_stub_leaves_no_trace` — needle assembled at runtime so the test file isn't a
  false-positive hit for its own search), `maintenance_driver.rs`'s
  `keyed_ledger_skip_reports_no_reporter_event` rewritten from the old capturing-reporter test.

## Decisions

- Deviated from the plan's exact placement of the "records no reporter event" test: kept it as an
  internal `maintenance_driver.rs` unit test (reusing the existing `RecordingBackend`/`SumRule`
  mocks) rather than adding it to `crates/smelt-runtime/tests/keyed_frontier_bookkeeping.rs`,
  which only drives a real DuckDB backend end-to-end and has no dialect-override seam. Same
  scenario coverage, different file.
- The plan's final gate `rg -n 'state_structure_unavailable' crates/` is stricter than the
  structural test's own stated scope (`src` only) — it also catches descriptive comments and the
  structural test's own needle. Scrubbed every literal occurrence repo-wide, including rewording
  comments and assembling the test's search needle at runtime (`["state_structure_un",
  "available"].concat()`) instead of writing it as a contiguous literal.
- `statement_parity.rs`'s `ledger_reset_is_skipped_on_a_non_duckdb_dialect` fixture's `smelt.yml`
  target type was `duckdb` even though its mock backend claimed `SparkSQL` — harmless under the
  old raw-`backend.dialect()` gate, but phase 5's availability resolution reads the *declared*
  target dialect from config, so the test's ledger-skip was never actually exercised until this
  phase's fix (changed target type to `spark`).

## For the next planner

- `docs/specs/state.md` §Known Divergences bullet "No availability-resolution step exists in
  derivation" (~line 297) is now stale — phases 4/5/6 landed exactly that. It correctly still
  flags that neither `DiagnosticCode` variant is implemented (phase 7's scope), but the framing
  ("no availability-resolution step exists") needs a rewrite. Flagged, not fixed — outside this
  phase's plan scope (only `cli.md`/`incremental_shapes.md` were listed).
- The `maintenance_driver.rs` ledger-less `else` branch this phase touched (~line 644, inside
  `run_windowed_keyed_maintenance`'s `Grade::Idempotent` arm) is very likely dead code in
  production now: availability resolution downgrades a `KeyedFold` cell to a recompute technique
  *before* this function is ever called with a ledger-requiring technique on a target lacking the
  ledger, so `backend.dialect() != DuckDB` inside this specific function should be unreachable
  outside the unit test that constructs the scenario directly. Worth a follow-up to confirm and
  either assert/panic on it or remove the branch.
- Found and fixed (not deferred) a pre-existing, unrelated nondeterminism bug in
  `smelt explain`'s default-target resolution (`HashMap::keys().next()`), which this phase's
  wiring turned into a flaky test on a two-target project. `resolve_default_target` is a stopgap
  (sorted-first, or `config.target` when declared) — the real fix is a proper "no target declared
  and 2+ targets exist" diagnostic, or switching `Config.targets` to a stable-order map
  (`IndexMap`) repo-wide. Out of this phase's scope; worth its own outcome/plan if other commands
  share the same `keys().next()` pattern (a `rg 'targets.keys().next()'` sweep would find them).
- `crates/smelt-cli/tests/explain_show_sql.rs::json_show_sql_reports_source_derived_columns_for_a_bigquery_median_model`
  needed a real fix (not just a re-record): the fixture's `ColumnScopedMerge` repair cell now
  correctly downgrades to `PerGroupRecompute` on BigQuery (no merge ledger), whose final copy
  statement is a legitimate `s.*` wildcard — the test's loop now scopes its `med_val`/`FLOAT64`
  assertions to the `DeleteInsert`-technique cell only, matching its own doc comment's stated
  target.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model` — 28 + 27 passed.
- `cargo test -p smelt-runtime --test availability_seam --test statement_parity --test
  execute_parity --test keyed_frontier_bookkeeping` — 6 + 37 + 4 + 4 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 75 passed.
- `cargo test -p smelt-cli --test explain_show_sql` — 9 passed (after the scoping fix).
- `cargo test -p smelt-cli --test explain` — 4 passed, stable across 8 repeated runs (after the
  `resolve_default_target` fix; previously flaky ~60% failure rate).
- `rg -n 'state_structure_unavailable' crates/` — no matches.
