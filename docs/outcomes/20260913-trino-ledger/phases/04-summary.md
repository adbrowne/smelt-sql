# Phase 4 summary — Wire the absence: downgrade, never refuse

**Shipped:**
- `smelt-db::queries::maintenance::backend_dialect_for` recognises `"trino"` → `SqlDialect::Trino`.
- `smelt explain` (`commands/explain.rs`), `smelt_runtime::profile::profiles_for_workspace`, and
  `smelt-ui`'s diagnostics endpoint (`build.rs`) no longer `?`-abort or drop a Trino-targeted model
  into `failures` on `smelt_backend::maintenance_dialect`'s `Err` — all three thread it as a
  `Result` and still build the availability-resolved report/profile/response.
- `smelt_runtime::diagnostics::build_model_diagnostics` takes
  `Result<MaintenanceDialect, UnsupportedMaintenanceDialect>`; `probe_plan::probe_plan_for_model`
  takes `Option<MaintenanceDialect>` and skips only the two probe families that need dialect-
  specific SQL (`declared_model_probes`, `append_only_posture_probes`) when `None`.
- New `diagnostics/preview.rs::unavailable_plan_cell_diagnostics`: every `ALL_TECHNIQUES` entry
  `NotApplicable`, naming the missing dialect. `build_admitted_statement_group`
  (`smelt-cli/src/explain.rs`) re-keyed to find by `admitted_technique` instead of scanning for
  `Admissibility::Admitted`, so it surfaces the named reason instead of a generic fallback.
- `execute/project/dry_run.rs`'s silent `continue` now calls `reporter.maintenance_warning`;
  `CliReporter` gained its first override of that method (`eprintln!`), which also makes
  pre-existing retention-downgrade warnings visible for the first time.
- Spec: `docs/specs/multi_backend.md` §"Parity contract" states the settled posture (no more
  "phase 4" forward-pointer), keeping the literal tokens the freshness gate asserts.

**Decisions:**
- No `MaintenanceDialect::Trino` variant added (per the plan) — statement rendering stays
  `20260913-trino-incremental`'s subject; this phase only decouples the *plan/report* from it.
- Widened `probe_plan_for_model`/`build_model_diagnostics` rather than threading `Option` all the
  way through every emitter (`declared_model_probes`, `build_technique_statements`, etc.) — those
  keep a required `MaintenanceDialect`; the `None`/`Err` case is absorbed at the two call sites
  named in the plan, confining the blast radius to ~6 call sites instead of ~30.
- Bumped `.claude/large-file-baseline.txt` (3 files, 7–14 lines each) and
  `.claude/hardening-baseline.txt` (`smelt-cli println` 189→190, an `eprintln!` substring match)
  with sign-off notes rather than trimming further — the growth is the `Result`/`Option`
  threading itself, not incidental bloat. See outcome.md decision log for full detail.

**For the next planner:**
- Probe-plan entries needing dialect-specific SQL (`timeseries.assert_monotonic`,
  `functional_dependencies:`, `bounded_domain:`) are silently omitted from `smelt explain`'s probe
  list on a Trino target rather than shown with a "not yet renderable" note — `ProbePlanEntry` has
  no reason field to carry that. Worth a follow-up once `20260913-trino-incremental` gives Trino
  probes SQL, or if this gap turns out to matter before then.
- `CliReporter::maintenance_warning` was previously unimplemented (silent no-op) — this phase's
  fix incidentally surfaces every pre-existing `maintenance_warning` call (e.g. retention
  downgrades from `execute/project/mod.rs`) in the terminal for the first time. Not regression-
  tested here beyond the new Trino dry-run case; worth a scan for surprising new stderr noise in
  existing fixtures if anyone reports it.
- Phase 5 (the two invariants as standing tests) can lean on `unavailable_plan_cell_diagnostics`
  and the `Result`/`Option` seams landed here rather than re-deriving the "no builder reachable"
  proof from scratch.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  full `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test maintenance_availability --test state_realisability_docs` — pass.
- `cargo test -p smelt-runtime --test availability_seam --test statement_parity` — pass.
- `cargo test -p smelt-cli --test trino_explain_downgrade --test trino_emission_spec_freshness` — pass.
- `cargo test -p smelt-core --test trino_docs_freshness` — pass.
- `bash .claude/scripts/large-file-check.sh` — pass (baseline bumped with sign-off note).
- `cargo test -p smelt-lsp --test example_workspaces` — pass (38/38).
