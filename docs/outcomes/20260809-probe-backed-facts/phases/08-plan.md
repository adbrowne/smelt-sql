# Phase 8 plan — surface: manifest `probes`, `smelt explain` rendering, docs-site

## Objective

Make probe activity visible to a user. Populate `ModelRunRecord.probes` from the four live
dispatch sites (declared and round-tripping since phase 4, but written empty by every site), and
render the model's probe set — declaration, named diagnostic, licensed cell, cadence, and the
per-run query cost — in `smelt explain <model>` (text and `--json`). Advances success criterion 5
(cadence controllable, probe cost visible in `smelt explain`) and completes criterion 4's
observability half (a skipped probe is *recorded as unverified*, never silently indistinguishable
from a checked one).

## Spec delta (made first, by the implement step)

`docs/specs/cli.md`:
- §"`smelt explain <model>` maintenance-plan report" — one paragraph: the report also prints the
  model's declared-fact probe set (`model_properties.md` §"Probe obligation"): per probe, the
  declared fact, the named diagnostic it raises, the licensed cell, the project cadence governing
  it (`smelt_yml.md` §"Top-level keys" `probes:`), and its cost — the one extra query each
  dispatched probe adds to a consuming run. A model declaring no probe-backed fact prints an
  empty probe set, not a missing section. The report stays offline: probe SQL is *built*, never
  executed, so no backend connection is made.
- §"`smelt explain --json` output schema" — the per-model maintenance report gains an
  append-stable `probes` array (§Constraints item 5):
  `{"fact": "...", "probe": "<DiagnosticCode>", "cell": "...", "cadence": "per_run"|"periodic"|"off", "cost": "<one line>"}`.

No `model_properties.md` / `run_state.md` edit: both already specify this surface
(`model_properties.md` line "Probe cost … is visible in `smelt explain`'s plan rendering";
`run_state.md` §"Run manifest" already defines the `probes` field and its two outcomes).

## Tests (red → green)

1. `crates/smelt-runtime/tests/probe_manifest.rs::manifest_records_dispatched_model_probes` — a
   real `execute_project` run of a model declaring `functional_dependencies:` writes a
   `ModelRunRecord.probes` entry with `fact`/`probe` set and `outcome: dispatched`.
2. `…::manifest_records_skipped_probes_under_cadence_off` — the same project with
   `probes: {cadence: off}` records the same probe with `outcome: skipped` (declaration trusted,
   not verified) rather than an empty array.
3. `…::manifest_records_source_posture_probe` — a model over an `append_only` source records the
   source-posture probe entry from the incremental-batch site.
4. `crates/smelt-cli/tests/explain_probes.rs::explain_text_report_lists_declared_probes` — the
   text report names the fact, the diagnostic code, the cadence, and a cost line for each probe.
5. `…::explain_json_carries_probes_array` — `--json` (with and without `--show-sql`) emits the
   `probes` array with the spec'd keys.
6. `…::explain_probe_set_is_offline` — the report builds for a model with declared facts without
   any backend connection (assert on the pure builder, no target credentials).
7. `crates/smelt-runtime/tests/model_probes.rs::monotonicity_probe_fires_named_diagnostic` (fix
   the existing test, phase 7's recorded production finding) — declare a `partition_column` the
   staged table actually has, and assert the failure is a genuine violation (message carries a
   non-zero violation count / sample keys), not a DuckDB binder error whose wrapped text happens
   to contain the diagnostic name.

## Tasks

1. Land the `cli.md` spec delta above.
2. Fix test 7 first (it is independent, and the current pass-for-the-wrong-reason masks the real
   detection path); keep the assertion strict enough that a binder error fails it.
3. Thread the records: add a per-model `Vec<ProbeRecord>` accumulator in `execute.rs`'s
   per-model scope, extend it from all four dispatch sites (`dispatch_declared_model_probes` and
   `dispatch_and_record_append_only_postures` at both the full-refresh site ~L3251/L3281 and the
   incremental-batch site ~L2754/L2794 — the batch loop is sequential, so a plain `mut` Vec is
   sound), and pass it into the `ModelRunRecord` construction for that arm (~L3099 incremental,
   ~L3412 full-refresh) instead of `Vec::new()`. The cumulative arm (~L2291) dispatches no
   probes today — leave it empty and say so in a comment.
4. Add `crates/smelt-runtime/src/probe_plan.rs`: `probe_plan_for_model(...) -> Vec<ProbePlanEntry>`,
   the pure, offline descriptor list explain renders. It must **reuse the dispatch owners**, never
   re-derive which declaration yields which probe: call `model_probes::declared_model_probes` with
   a symbolic scope select (e.g. `SELECT * FROM {{model}}`) and map each `ProbeContext` to an
   entry; for the source-declared probes reuse `source_probes`'s existing consumed-source
   resolution and eligibility predicate (make it `pub` if needed) for `mutation_profile.kind:
   append_only`, and the consumed sources' `key_recurrence` / `referential_integrity`
   declarations for the two plan-driven registry rows. Each entry carries `fact`, `probe`
   (diagnostic code), `cell`, and a one-line `cost`.
5. Render it: extend `build_maintenance_plan_report` with a `Probes (N):` section (cadence line
   from the project `probes:` config, one block per entry), and add the `probes` array to
   `ExplainMaintenanceJson`; wire both from `commands/explain.rs`, which already has `config`,
   `model.metadata`, and `source_infos` in scope.
6. docs-site: `reference/smelt-yml.md` — document the `probes:` key (`cadence`, `every_n_runs`),
   which has no user-doc coverage at all today; `reference/smelt-explain.md` — the new probes
   section plus the JSON field; `reference/state.md` — the manifest's `probes` field and its two
   outcomes.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test probe_manifest --test model_probes --test source_probes --test probe_dispatch`
- `cargo test -p smelt-cli --test explain_probes --test explain_model --test explain_maintenance --test explain_show_sql`
- `cargo test -p smelt-logical --test probe_obligation` (registry unchanged — surface-only phase)
- `cargo test -p smelt-runtime --test statement_parity` and
  `cargo test -p smelt-cli --test maintenance_conformance` (no emitter or dispatch change)

## Commit message

`feat(probes): record dispatched/skipped probes on the run manifest and render the probe set in smelt explain`
