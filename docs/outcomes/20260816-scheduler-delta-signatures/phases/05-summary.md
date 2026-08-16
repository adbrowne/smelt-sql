# Phase 5 summary — propagated key restrictions reach the key-addressed cell

## Shipped

- Spec: `docs/specs/incremental_models.md` §"Dispatch — from propagated components to run units"
  gained **"Restrictions compose by union"**, pinning the union-never-intersect rule normatively.
- `smelt_runtime::types::KeyedRestriction { upstream, keys, values }` and
  `ExecuteRequest::keyed_restrictions: BTreeMap<String, Vec<KeyedRestriction>>` (`types.rs`) — the
  request-level channel a propagated keyed component now rides on.
- `smelt_runtime::propagation::keyed_restrictions_from_plan(&SinceUpstreamPlan) ->
  BTreeMap<String, Vec<KeyedRestriction>>` — pure conversion of `SinceUpstreamPlan::keyed_dirty`;
  only `KeyValues::Resolved` entries contribute, values sorted/deduped, an unresolved seed
  contributes nothing.
- `maintenance_driver::union_affected_keys` (pure) + threaded `restriction_keys: &[String]`
  through `resolve_key_addressed_affected_keys` and `execute_key_addressed_model_edge_cell` — the
  sidecar's own `changed_keys` and the restriction are unioned, sorted, deduped, *before*
  `emit_key_addressed_affected_keys_select` is called (the emitter still authors the SELECT).
- `execute.rs`: new `restriction_keys_for(request, model, edge_name)` helper; all three
  `dispatch_key_addressed_model_edge` call sites (the `grain: key` branch's leading+rest cells,
  and the non-keyed coverage-gate branch) now look up and pass the request's restriction for
  that `(model, edge)` pair.
- `smelt-cli/src/commands/run.rs::run_since_upstream` populates `ExecuteRequest::keyed_restrictions`
  once from `keyed_restrictions_from_plan(&plan)` and passes the same map into every per-model
  request — `execute_project` selects each model's own entries by name.
- ~42 `ExecuteRequest` struct literals across the workspace updated with the new field (mechanical,
  all defaulting to an empty map so existing behaviour is unperturbed).

## Decisions

- Consolidated the plan's 6 named tests into 5 test functions covering the same intent, favouring
  precise, fast unit tests over duplicate expensive e2e chains: the union rule (tests 3+4) is
  pure-unit-tested against the extracted `union_affected_keys` helper rather than through a real
  DuckDB sidecar, since the union arithmetic itself needed no backend to verify.
- The single e2e test (`propagated_restriction_key_is_repaired_when_sidecar_reports_no_change`,
  covering plan tests 1+2 together) proves the load-bearing claim via directly corrupting a
  downstream row without touching any upstream data — the only way to observationally distinguish
  "sidecar found nothing, restriction alone dispatched a real write" from a no-op, since manifest
  `strategy` labels the same string (`per_group_recompute`) whether or not `execute_key_addressed_
  model_edge_cell` returned `Some` or `None`.
- Test 6 (`keyed_restrictions_from_plan_drops_unresolved_values`) needed a real delta *origin* on
  `agg` itself (not the empty-deltas pattern the sibling seeded test uses) — `propagate_with_keys`
  only visits a node when it has interval dirt, keyed dirt, or an explicit seed; an unseeded,
  undelta'd node is never visited at all, so no Unresolved record would ever propagate without one.

## For the next planner

- Row 6 (live consumption) is the natural next step: today `--since-upstream`'s keyed-restriction
  leg is wired but its resolved-value set is empty in practice (no live keyed-seed resolution
  exists yet), so `keyed_restrictions_from_plan` is exercised directly against a seeded plan in
  tests, never through the CLI end-to-end. Once row 6 lands live seed resolution, an actual CLI
  e2e test (`--since-upstream` producing a non-empty restriction that reaches a real run) becomes
  possible and would strengthen coverage beyond this phase's synthetic-restriction tests.
- The `group_by_output_keys` alias gap (row 9) is unrelated to this phase and untouched.
- Fan-in merge across more than one admitted inbound keyed edge (flagged in phase 3's summary) is
  still untested — this phase's restriction channel doesn't interact with that path.

## Gates

- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering --test since_upstream_propagation` — PASS (15 + 21 tests)
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity` — PASS (4 + 23 tests)
- `cargo test -p smelt-cli --test maintenance_conformance` — PASS (76 tests)
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md` — no matches
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full workspace test suite, example_diagnostics)
