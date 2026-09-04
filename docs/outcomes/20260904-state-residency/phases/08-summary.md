# Phase 8 summary — `state.mode` honoured in `execute_project`

## Shipped

- `crates/smelt-state/src/file_store.rs`: `StateArtifact` enum + pure
  `state_artifacts_written(StateMode)` table (the single source of truth for
  the posture consequence table); `FileStore::with_state_mode(project_dir,
  target, mode)` — the gated constructor. Every `save_*`/`load_*`/`init`/
  `lock` is gated on `self.allows(artifact)`/`self.allows_any()`: a denied
  write is a no-op that touches no path, a denied read returns the artifact's
  default without touching disk. `FileStore::new` is unchanged (permissive —
  the read/tooling constructor for `history`/`status`/`diff`/`migrate`).
- `crates/smelt-runtime/src/execute.rs`: the run pipeline's store is now
  built via `FileStore::with_state_mode(project_dir, &request.target,
  config.state.mode)`; `--resume` under `state.mode: stateless` refuses by
  name before it ever reaches `load_runs`.
- `crates/smelt-cli/src/commands/history.rs`, `status.rs`: an empty result
  under `stateless` now says the posture is why, instead of reading as "no
  runs yet".
- `docs/specs/state.md`: two inventory rows (source-mutation baselines,
  migration approvals) that existed in code but not the table; the
  `intervals` row's write-set list gained both; a `--resume`-under-stateless
  clause in §"The optionality rule".
- New tests: `smelt-state` unit tests (`written_artifacts_match_the_posture_
  table`, `stateless_store_writes_nothing`, `intervals_store_denies_snapshot_
  store`, `stateless_loads_return_defaults_over_stale_files`); `smelt-cli/
  tests/state_posture.rs` (5 real-binary CLI tests); `smelt-runtime/tests/
  state_posture_seam.rs` (structural: `execute.rs` must call `with_state_
  mode`, never bare `new`); `contract_deferral_skip_e2e.rs::stateless_
  deferral_cell_folds_every_run`.

## Decisions

- `FileStore::new` stays permissive rather than gated — `smelt history`/
  `status`/`diff`/`migrate` must see whatever a run actually wrote,
  independent of the *current* invocation's posture.
- The posture gate lives inside `FileStore` (one seam, checkable by a single
  unit test) rather than as guards at ~15 call sites in `execute.rs`.
- `--resume`'s posture check sits before `load_runs`, not as a special case
  of the existing "no partially-failed run" error — the spec requires the
  message to name the posture, which a missing-manifest message can't do
  faithfully (a stateless project's absence isn't "no failed run", it's "no
  history at all").
- No new production code was needed to make `contract.deferral` degrade
  correctly under `stateless`: `run_license`'s existing `None`-frontier ⇒
  `Run` fallback (already covering "not yet run") transparently also covers
  "posture excludes this state" once `FileStore` returns defaults. This
  phase adds a test locking that behavior, not new logic.
- `environments_run_adds_the_snapshot_store`'s CLI-level test asserts a
  superset relationship (artifact *kinds* intervals writes, normalized past
  run-id-stamped filenames) rather than the full exact-set from the spec
  table, because — see below — nothing in production actually writes the
  snapshot store yet.

## For the next planner

- **Real gap, not a phase-8 regression:** `smelt_state::snapshot_store::
  SnapshotStore::save_snapshot_store` has **no production caller** anywhere
  in `smelt-runtime`/`smelt-cli` — virtual environments (`state.mode:
  environments`'s distinguishing feature) never actually persists a
  snapshot under the real run pipeline today. `crates/smelt-fingerprint/src/
  reuse.rs` reads `StateMode::Environments` but nothing writes the store it
  would consult. This predates phase 8; phase 8 only made it *visible*
  (the exact-set CLI test couldn't be written as specified). Not fixed here
  — it's virtual-environments implementation work, out of this outcome's
  scope, but the next planner touching `virtual_environments.md` should
  know the snapshot store is currently decorative.
- **Wide fixture-sweep fallout, now closed:** honouring `state.mode` for
  real (previously every `FileStore` write was unconditional regardless of
  the config) turned out to affect far more of the test suite than the
  plan's named list — 9 additional files beyond the plan's task 6 list
  needed `state: mode: intervals` added to their fixture `smelt.yml`s
  (`smelt-cli/tests/{run_report,failure_summary,list_clean,migrate_apply,
  migrate_plan,explain_definition_delta,partition_residue_probes,
  definition_delta_gate,e2e/{declared_fact_probe_firing,full_refresh_escape_
  rebuild,schema_evolution_incremental,schema_roundtrip}}.rs`,
  `smelt-runtime/tests/{statement_parity,technique_lowering}.rs`,
  `smelt-maintenance-testkit/src/render.rs`). All are now green; a full
  `cargo test` workspace run confirms nothing else regressed. Any *new*
  test fixture added after this phase that asserts run history, schema
  tracking, or interval state must declare `state.mode: intervals` (or
  `environments`) explicitly — the default is now genuinely `stateless`.
- `.claude/hardening-baseline.txt`'s `smelt-cli println` count moved
  172→174 (the two new posture-naming messages in `history.rs`/`status.rs`,
  legitimate user-facing CLI output) — ratcheted via `--update`, committed.
- Phase 9 (the `.smelt/`-deletion conformance-gate leg) can now lean on
  `state.mode: stateless` as a real, load-bearing posture rather than an
  aspirational one.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both
  feature sets, full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-state --lib` — 306 passed.
- `cargo test -p smelt-cli --test state_posture --test resume --test
  run_report --test incremental --features duckdb` — 5+5+3+48 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --features
  duckdb` — 75 passed.
- `cargo test -p smelt-runtime --test execute_parity --test
  statement_parity` — 37 passed.
- `cargo test -p smelt-lsp --test example_workspaces` — 35 passed.
