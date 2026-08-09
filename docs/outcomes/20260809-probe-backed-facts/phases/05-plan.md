# Phase 5 plan — live dispatch of the three model-scoped probes at the pre-write site

## Objective

Wire `emit_monotonicity_probe`, `emit_functional_dependency_probe` and `emit_bounded_domain_probe`
into real runs: one owner in `smelt-runtime` builds the declared-probe set from a model's metadata,
dispatches each through `probes::dispatch_probe` over the run's compiled SQL as `scope_select`,
**before** the write, and turns a violation into the registry's named diagnostic. Advances success
criteria 2 (probes dispatched, not just emitted) and 4 (firing → named diagnostic carrying fact,
cell, remedy), and flips three registry rows `built (unwired)` → `built`.

## Spec delta

`docs/specs/model_properties.md`:
- §"Probe obligation" registry — Status cell of the `timeseries.assert_monotonic`,
  `functional_dependencies:` and `bounded_domain:` rows: `built (unwired)` → `built`.
- §Known Divergences, the "four of seven registry rows have an emitter but no live dispatch" entry —
  rewrite to name only the append-only posture probe as unwired (phase 6), and record that the three
  model-scoped probes now dispatch at the pre-write site over the run's compiled SQL, governed by
  the `probes:` cadence policy. Keep the `unique_key`/`delta_identity` `not-yet` note unchanged.

No user-visible surface change beyond a new failure mode already specified in §"Probe obligation";
`docs-site/` rendering of probes belongs to phase 8.

## Tests

New `crates/smelt-runtime/tests/model_probes.rs` (real DuckDB, mirroring `probe_dispatch.rs`'s harness):
1. `fd_probe_fires_named_diagnostic` — a model declaring `customer_id → region` over data with two
   regions for one customer errors with `DeclaredFunctionalDependencyViolated`, the sample keys, the
   licensed cell and the registry remedy.
2. `fd_probe_holds_on_conforming_data` — same declaration, conforming data → `Ok`.
3. `bounded_domain_probe_fires_named_diagnostic` — distinct count above `max_cardinality` →
   `DeclaredBoundedDomainExceeded`.
4. `monotonicity_probe_fires_named_diagnostic` — `assert_monotonic: true` over rows whose event time
   goes backwards within a partition → `DeclaredMonotonicityViolated`.
5. `undeclared_model_dispatches_no_probes` — metadata with none of the three declarations issues no
   probe SQL (assert the returned dispatch record list is empty).
6. `assert_monotonic_false_dispatches_no_monotonicity_probe` — a timeseries model without the
   declaration is not probed for monotonicity.
7. `cadence_off_skips_declared_probes` — violating data + `ProbeCadence::Off` → `Ok` (policy skip
   trusts the declaration; distinct from unbuildable).
8. `multiple_functional_dependencies_each_dispatch` — two `functional_dependencies:` entries produce
   two probes; the second one's violation still fires.

New `crates/smelt-cli/tests/e2e/declared_fact_probe_firing.rs` (end-to-end via the real pipeline,
following the project-fixture harness used by the neighbouring e2e tests):
9. `violating_fd_fails_the_run_before_any_write` — a `table` model with a violated
   `functional_dependencies:` declaration fails the build with `DeclaredFunctionalDependencyViolated`
   and leaves the target table absent (probe precedes `create_table_as`).
10. `violating_bounded_domain_fails_an_incremental_batch_before_its_write` — same for the
    incremental partition write path; the target's pre-run contents are unchanged.
11. `probes_off_lets_the_violating_run_write` — the same project with `probes: {cadence: off}`
    builds successfully, proving the cadence policy governs the new sites.

Existing gates that must stay green as oracles: `probe_obligation` (registry status/emitter
consistency after the spec flip), `example_diagnostics` + e2e example builds (the three
`examples/*_declared` fixtures now dispatch real probes on every build and must hold).

## Tasks

1. Make the spec delta above.
2. Add `crates/smelt-runtime/src/model_probes.rs`: `declared_model_probes(metadata, timeseries) ->
   Vec<DeclaredProbe>` (pure — fact, probe code, remedy, emitter call), and
   `dispatch_declared_model_probes(backend, policy, model_name, metadata, timeseries, scope_select,
   cell, dialect) -> Result<Vec<ProbeRecord-ish>, BackendError>` calling `probes::dispatch_probe` per
   probe. Probe codes/remedies come verbatim from the registry rows.
3. Map `ProbeVerdict::Violated` to `BackendError::ExecutionFailed` whose message opens with the
   registry diagnostic code, names the model, the violation count and `sample_keys`, and ends with
   `probes::probe_violation_suffix(ctx)` (cell + remedy). Never a warning, never a continue.
4. Export the module from `crates/smelt-runtime/src/lib.rs` alongside `probes`.
5. Call it in `execute.rs` at the full-refresh/standard pre-write site (immediately after
   `reporter.model_compiled` at the `None => // Full refresh` arm, before the materialization write),
   with `scope_select = &compiled.sql` and a cell label naming the materialization.
6. Call it in `execute.rs` at the incremental batch pre-write site (after `reporter.model_compiled`
   in the batched partition arm, before the DELETE+INSERT / restricted recompute), with the batch's
   `compiled.sql` as scope and a cell label naming the partition region.
7. Both call sites build the policy with the existing `probe_policy_for_model(config, prior_runs,
   &plan.name)` — no new plumbing.
8. Write the tests above red first; then implement until green.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test model_probes --test probe_dispatch --test statement_parity --test execute_parity`
- `cargo test -p smelt-logical --test probe_obligation`
- `cargo test -p smelt-cli --test e2e --test example_diagnostics --test maintenance_conformance`
- `cargo test -p smelt-lsp --test example_workspaces`

## Commit message

`feat(probes): dispatch the declared model-scoped probes (FD, bounded-domain, monotonicity) before every write`
