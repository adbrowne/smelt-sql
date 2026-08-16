# Phase 6 summary — live dispatch of the source append-only posture probe

**Shipped:**
- `crates/smelt-logical/src/maintenance/emit.rs`: `AppendOnlyBaselinePartition` gained
  `check_fingerprint: bool` (4th `VALUES` column); the emitter's predicate is now
  `current_count < recorded_count OR (check_fingerprint AND fingerprint IS DISTINCT FROM …)`.
  New `emit_append_only_baseline_snapshot(source_table, partition_column, digest_columns,
  dialect)` is the extracted shared per-partition current-state `SELECT`.
- `crates/smelt-state/src/source_postures.rs` (new): `SourcePostureStore`,
  `SourcePosturePartition`, `closed_baseline()` (frontier gate). Wired into
  `crates/smelt-state/src/file_store.rs` (`load_source_postures`/`save_source_postures`,
  `.smelt/targets/<target>/source_postures.json`) — deliberately absent from
  `migrate_legacy_layout_locked`'s list (a brand-new file, not a legacy pre-versioning one).
- `crates/smelt-runtime/src/source_probes.rs` (new): `append_only_posture_probes` (pure
  builder) + `dispatch_and_record_append_only_postures` (dispatch/refresh driver). Wired at
  both pre-write sites in `crates/smelt-runtime/src/execute.rs` (full-refresh arm, incremental
  batch loop), under `state_io_lock`, mirroring `model_probes`'s shape.
- Spec: `docs/specs/model_properties.md` §"Probe obligation" row flips to `built`; Known
  Divergences replaces the "unwired" note with the `mutation_profile.lateness` limitation.
  `docs/specs/sources.md` §Known Divergences updated to say `SourceMutationProfileViolated`
  now dispatches.
- Tests: `crates/smelt-logical/tests/probe_execution.rs` (3 new frontier-gate cases),
  `crates/smelt-state/src/source_postures.rs` (in-module, 3 cases),
  `crates/smelt-runtime/tests/source_probes.rs` (new, 4 cases),
  `crates/smelt-runtime/tests/statement_parity.rs` (1 new byte-identity case),
  `crates/smelt-cli/tests/e2e/declared_fact_probe_firing.rs` (2 new cases),
  `crates/smelt-logical/tests/probe_obligation.rs` (renamed/updated: no `built (unwired)`
  rows remain).

**Decisions:**
- An eligible source with no recorded baseline builds a `SourcePostureAction::Establish`
  (not `Verify`): its first observation is unconditionally recorded as the baseline under the
  same cadence gate, rather than never establishing one. Discovered mid-implementation via the
  real-DuckDB test harness; `DeclaredSourceProbe` carries an `action: SourcePostureAction` enum
  (`Verify { sql, snapshot_sql }` / `Establish { snapshot_sql }`) instead of a flat SQL field.
- `maintenance_conformance`'s generative `AppendLateRow` schedules legitimately append into an
  already-closed partition — the exact `mutation_profile.lateness` gap this phase documents as
  a known divergence — so `smelt-maintenance-testkit::render::render_smelt_yml` now declares
  `probes: {cadence: off}`: that harness's job is maintenance-technique equivalence (its own
  S-restricted oracle), not probe-firing behaviour, which has dedicated coverage elsewhere.
- Source address convention for `source_postures.json` keys matches `landed_deltas.json`'s
  existing bare-name convention (`sources.raw.events` → `raw.events`), computed locally in
  `source_probes.rs` rather than reused from `build_maint_source_facts` (kept the module
  independent of a `smelt-logical`-shaped intermediate).

**For the next planner:**
- Phase 7 (conformance recipes): the append-only violation scenario (mutate a closed
  partition's content in place; count-decrease on any partition) is now available for the
  fact-violation recipe pool — `declared_fact_probe_firing.rs`'s two new e2e tests are a
  worked example of staging it.
- Phase 8 (`ModelRunRecord.probes` population): this phase's dispatch sites
  (`dispatch_and_record_append_only_postures`) return `Vec<ProbeRecord>` exactly like
  `model_probes`'s — still unconsumed by any manifest write, now a 4th call site's worth of
  the same unresolved item phase 5 flagged.
- Out of scope, discovered: `mutation_profile.lateness` interaction with the frontier gate is
  a real, exercised gap (not hypothetical) — the conformance harness had to route around it.
  A future widening of this probe (or the frozen-horizon work it's tracked under,
  `docs/plans/20260715-composed-axes-conditional-maintenance.md`) should consult the source's
  declared lateness margin before treating a closed-partition append as a violation.

**Gates:**
- `bash .claude/scripts/verify-phase.sh --fast` — PASS (fmt, clippy zero-warnings,
  example_diagnostics).
- `cargo test -p smelt-runtime --test source_probes --test model_probes --test probe_dispatch
  --test statement_parity --test execute_parity` — PASS.
- `cargo test -p smelt-logical --test probe_obligation --test walk_coverage` — PASS.
- `cargo test -p smelt-state` (source_postures lives in-module) — PASS.
- `cargo test -p smelt-cli --test e2e --test maintenance_conformance --test
  example_diagnostics` — PASS (maintenance_conformance required the `probes: {cadence: off}`
  fix above; 59/59 green after).
- Full `bash .claude/scripts/verify-phase.sh` (fmt + clippy + full `cargo test` +
  example_diagnostics) launched in background for final confirmation before commit.
