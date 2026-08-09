# Phase 4 plan — probe policy: `probes:` in `Config`, cadence decision, shared dispatch + named firing diagnostic

## Objective

Give the run driver one owner for *whether* a probe runs and *what happens when it fires*:
`probes:` becomes a real `Config` field, a pure cadence decision governs dispatch, and one
`smelt-runtime` helper executes the probe, reads the single `violation_count`/`sample_keys`
contract, and fails the run with a named diagnostic naming the violated fact, the affected
cell and the remedy. The two already-dispatched probes (recurrence-bound, count-preservation)
are re-routed through it. Advances success criteria 4 and 5; unblocks 6 by giving phases 5–6
a dispatch call to reuse rather than re-invent per probe.

## Spec delta (implement step makes these edits first)

- `docs/specs/smelt_yml.md` — §Known Divergences: drop "`probes:` is specified and
  unimplemented"; §Semantics 10 gains: a skipped dispatch (`off`, or a `periodic` run that is
  not the nth) is *recorded per model on the run manifest* as `unverified this run`, and a
  skip means the declaration is **trusted** for that run, not that the technique it licenses is
  abandoned.
- `docs/specs/model_properties.md` §"Probe cadence" — distinguish the two non-dispatch cases:
  a **policy skip** (cadence) trusts the declaration and records it unverified; a **probe that
  cannot be built** stays fail-closed exactly as today (recurrence route 3 refuses; the RI
  route drops the restriction and widens). State that a firing probe's diagnostic carries the
  violated fact, the maintenance cell it was licensing, and the remedy text from the registry.
- `docs/specs/run_state.md` §"Run manifest" — new optional per-model `probes` array
  (`fact`, `probe`, `outcome: dispatched | skipped`), serde-defaulted empty for older manifests.

## Tests (red first)

- `smelt-core` `config.rs` unit tests:
  - `test_probes_defaults_to_per_run` — absent `probes:` yields `ProbeCadence::PerRun`.
  - `test_probes_periodic_requires_positive_every_n_runs` — `periodic` without `every_n_runs`,
    or with `0`, is a configuration error, never a silent default.
  - `test_probes_rejects_unknown_cadence_and_unknown_fields` — fail loud, not `Unknown`.
- `crates/smelt-logical/tests/probe_cadence.rs` (new, pure):
  - `per_run_dispatches_every_run`
  - `off_never_dispatches`
  - `periodic_dispatches_on_the_first_run_then_every_nth` — ordinal 0 always verified.
- `crates/smelt-runtime/tests/probe_dispatch.rs` (new, real DuckDB):
  - `holding_probe_reports_held_and_no_error`
  - `violating_probe_fails_with_named_diagnostic_fact_cell_and_remedy` — message contains the
    registry code, the declaration, sample keys, the licensed cell, and the remedy sentence.
  - `probe_result_missing_violation_count_fails_loud` — a malformed probe row refuses rather
    than being read as "no violation".
  - `off_cadence_skips_dispatch_and_executes_no_sql` — helper returns `Skipped`, backend sees
    no probe query.
  - `recurrence_probe_under_off_cadence_runs_without_dispatch` — a route-3 declared merge
    completes (declaration trusted, recorded unverified) instead of failing closed.
  - `unbuildable_probe_still_fails_closed_under_per_run` — the policy-skip path did not weaken
    the probe-unavailable path.
- `smelt-state` unit test `probe_records_default_empty_on_legacy_manifest` — round-trips a
  manifest written without the new field.
- Existing suites must stay green unchanged: `locality_route3_recurrence_check`,
  `technique_lowering` (RI tests), `statement_parity`.

## Tasks

1. `smelt-core/src/config.rs`: `ProbesConfig { cadence: ProbeCadence }`,
   `ProbeCadence::{PerRun, Periodic { every_n_runs: u32 }, Off}`, `deny_unknown_fields`,
   validation, `Config.probes: ProbesConfig` defaulting to `per_run`.
2. `smelt-logical/src/maintenance/probe_cadence.rs`: pure
   `should_dispatch(cadence, run_ordinal) -> ProbeDispatch::{Dispatch, Skip(SkipReason)}`;
   re-exported from the maintenance module (probe policy is maintenance-plan data, not runtime
   ad-hockery).
3. `smelt-runtime/src/probes.rs` (new): `ProbePolicy { cadence, run_ordinal }`;
   `dispatch_probe(backend, policy, ProbeContext { fact, probe_code, model, cell, remedy },
   statement) -> Result<ProbeVerdict>` — executes, parses the one-row contract (fail loud on a
   missing/unparseable `violation_count`), returns `Skipped | Held | Violated { count,
   sample_keys }`; `probe_violation_error(...)` builds the named message.
4. Build `ProbePolicy` once per run in `execute.rs` from `config.probes` plus the model's prior
   run count (`smelt_state::history::HistoryQuery::for_model` over `file_store.load_runs`),
   and thread it to the maintenance driver.
5. Re-route `run_windowed_keyed_maintenance`'s recurrence-bound dispatch and
   `execute_delete_insert_with_delta_restriction`'s count-preservation dispatch through
   `dispatch_probe`, preserving today's message prefixes and adding cell + remedy text; keep
   both fail-closed paths for *unbuildable* probes exactly as they are.
6. Record each probe's dispatched/skipped outcome on the per-model `ModelRunRecord.probes`
   (`smelt-state`), written by the existing manifest writer.
7. Update the probe registry's cadence/status prose in `model_properties.md` for the two wired
   rows (now cadence-governed) and the Known Divergences line about cadence applying only to
   `key_recurrence`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test probe_cadence --test probe_obligation`
- `cargo test -p smelt-runtime --test probe_dispatch --test locality_route3_recurrence_check --test technique_lowering --test statement_parity`
- `cargo test -p smelt-cli --test maintenance_conformance --test example_diagnostics`

## Commit message

`feat(probes): project-level probe cadence policy and a single-owner probe dispatch that fails the run with a named diagnostic`
