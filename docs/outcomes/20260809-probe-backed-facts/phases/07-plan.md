# Phase 7 plan — conformance recipes: violated-fact scenarios caught by probes

## Objective

Advance success criterion 6: the standing conformance gate gains a **fact-violation recipe
pool** — one recipe per `built` row of the probe registry — where each recipe stages a real
project, feeds it conforming data (run succeeds, output equals the full-refresh oracle) and
then violating data (run fails with the registry's named diagnostic, before any write). A
spec-parsed coverage assertion makes an unrecipe'd `built` row a test failure, so the pool
cannot silently fall behind the registry. Criterion 6's "caught by its probe, **not** by wrong
output" is made checkable by a third leg: where the violation is end-state observable, the same
violating data under `probes: {cadence: off}` must produce output that *differs* from the
oracle — proving the probe is load-bearing rather than decorative.

## Spec delta

No behaviour change. One docs-only edit: `docs/specs/model_properties.md` §References →
**Tests** gains the fact-violation conformance pool
(`crates/smelt-cli/tests/maintenance_conformance/fact_violations.rs`) alongside the existing
entries, so the registry's verification story is discoverable from the spec.

## Tests

New module `crates/smelt-cli/tests/maintenance_conformance/fact_violations.rs` (registered in
`main.rs`; the target is already `#![cfg(feature = "duckdb")]`). Recipes are table-driven data,
one per `built` registry row, staged and driven in-process through `smelt_runtime::execute_project`
(the `link_c_harness`/`base_request` shape `gate.rs` uses), each with an explicit
`end_state_observable: bool` + reason.

- `every_built_registry_row_has_a_violation_recipe` — parses the §"Probe obligation" table from
  `docs/specs/model_properties.md` (same extraction shape as
  `crates/smelt-logical/tests/probe_obligation.rs`); every row whose Status is `built` must have
  a recipe keyed by its diagnostic name, and every recipe must name a `built` row. Coverage
  ratchet — this is the gate that keeps the pool honest.
- `conforming_data_runs_clean_and_matches_the_oracle` — for every recipe, the conforming feed
  succeeds and the target's contents multiset-equal the full-refresh oracle (recipe is valid
  and the declaration's technique is actually exercised, not vacuous).
- `violated_fact_fails_before_any_write` — for every recipe, the violating feed exits non-zero
  with the registry's diagnostic name in the output, and the target table's contents are
  byte-for-byte what they were before the run (no partial write).
- `violation_is_end_state_observable_when_probes_are_off` — for every recipe with
  `end_state_observable: true`, the same violating feed under `probes: {cadence: off}` writes,
  and its output differs from the full-refresh oracle. Recipes marked `false` are counted and
  printed as explicit skips with their reason (the `ProbeOutcome::Skipped` convention in
  `smelt-maintenance-testkit::probes`), never silently absent.

Recipe pool (6 = the `built` rows):

| Diagnostic | Shape to stage | Starting point |
|---|---|---|
| `DeclaredFunctionalDependencyViolated` | `table` model, `functional_dependencies:`, two regions for one key | `e2e/declared_fact_probe_firing.rs::stage_fd_workspace` |
| `DeclaredBoundedDomainExceeded` | incremental batch, `bounded_domain:` exceeded in one batch | `e2e/declared_fact_probe_firing.rs::stage_bounded_domain_workspace` |
| `SourceMutationProfileViolated` | `append_only` source, closed partition mutated between runs | `e2e/declared_fact_probe_firing.rs::stage_append_only_workspace` |
| `DeclaredMonotonicityViolated` | `timeseries` + `assert_monotonic`, out-of-order event time within a partition | `crates/smelt-runtime/tests/model_probes.rs` |
| `SourceCountPreservationViolated` | enrichment inner join, declared `referential_integrity`, dimension key missing | `crates/smelt-cli/tests/e2e/events_deduped_redelivery_equivalence.rs` |
| `KeyedRecurrenceBoundViolated` | keyed model, declared `key_recurrence.window`, key recurring outside it | `crates/smelt-runtime/tests/locality_route3_recurrence_check.rs` |

## Tasks

1. Add `mod fact_violations;` to `crates/smelt-cli/tests/maintenance_conformance/main.rs`.
2. Define the recipe type: name, diagnostic, project staging fn, conforming feed, violating
   feed, oracle SQL, `end_state_observable: bool` + reason. Staged projects declare `probes:`
   explicitly (`per_run` for legs 2–3, `off` for leg 4) — the testkit's `render_smelt_yml`
   default of `cadence: off` does not apply here since these projects are hand-staged.
3. Write the three data-driven recipes lifted from `declared_fact_probe_firing.rs` (FD,
   bounded_domain, append-only) as in-process recipes; leave the existing binary-level e2e tests
   untouched (they prove the CLI wiring; this pool proves the pool).
4. Write the three new recipes (monotonicity, count-preservation, recurrence-bound), each
   red-first: assert the firing leg before the project is correct, confirm it fails for the
   right reason.
5. Implement the four test fns above over the pool; wire the spec-registry coverage parse.
6. Record the skip reasons for any `end_state_observable: false` recipe in the module doc
   comment, one line each.
7. Update `docs/specs/model_properties.md` §References → Tests.
8. Write `phases/07-summary.md`.

If a recipe's firing leg cannot be made to fire against real DuckDB, that is a **production
finding**, not a recipe to weaken: record it in the summary and in the registry's §Known
Divergences with the row's Status left at `built` and the gap named. Do not delete the recipe.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test maintenance_conformance` (the pool plus the existing gate;
  keep the added wall-clock under ~60s — recipes are single-model projects, not generated DAGs)
- `cargo test -p smelt-cli --test e2e` (the three lifted e2e tests still pass unchanged)
- `cargo test -p smelt-logical --test probe_obligation`
- `cargo test -p smelt-runtime --test statement_parity --test model_probes --test source_probes`

## Commit message

`test(probes): fact-violation recipe pool in the conformance gate, one per built registry row`
