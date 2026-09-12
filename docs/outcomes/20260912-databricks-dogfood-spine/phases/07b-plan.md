# Phase 7b plan — a realisable route for the key-addressed model-edge cell on Delta

## Objective

`gold.events_enriched` is the one model that fails in every Databricks run — full refresh and
all three incremental windows alike — because its `Technique::PerGroupRecompute` cell, derived
over a key-addressed model edge, needs a group-grain fingerprint-sidecar diff that Spark/Delta
does not realise, and the gap surfaces as an **execution-time** `BackendError::unsupported`
rather than a plan-derivation downgrade. This phase makes the need part of availability
resolution, so the cell downgrades to `DeleteInsert` with a recorded
`MaintenanceStateDowngraded` instead of aborting the model. It advances criteria 6 (whole model
set completes) and, by unblocking `marts.star_growth`'s skip, gets the run to 16/16 — the state
rows 8 and 9 read.

## The decision this phase makes

The outcome row offers two routes: realise the sidecar on Delta, or downgrade at plan
derivation. **Downgrade.** Realising the sidecar means Spark emitters in
`smelt-state/src/ddl_spark.rs`, a capability flip, and new backend seams — a correctness
feature for the follow-on `databricks-correctness` outcome, not the "only way a run completes
at all" fix this outcome's Out-of-scope section licenses. Downgrading is also the shape the
degradation contract already specifies (`state.md` §"The degradation contract"): a cell needing
an unrealisable structure becomes the cheapest recompute-family technique preserving the
equivalence invariant. `DeleteInsert`, not `PerGroupRecompute` — the key-addressed cell's
*whole* per-group route is what the missing sidecar denies, so recomputing the region is the
only honest fallback.

## Root cause (verified, not assumed)

`availability::required_state_structure` is keyed on `Technique` alone and returns `None` for
`PerGroupRecompute` (`crates/smelt-logical/src/maintenance/availability/state_structure.rs:119`),
so `resolve_availability` never considers this cell. The sidecar requirement is instead
re-discovered at run time by
`maintenance_driver/key_addressed/mod.rs:124`'s `!supports_fingerprint_sidecar` bail. That is a
second source of truth for a plan fact — the thing maintenance-plan purity forbids. Note the
requirement is a property of the **cell**, not the technique: only a `PerGroupRecompute` cell
carrying a `key_scope` whose `discovery` is `UpstreamKeyed` or `DownstreamGrainOverUpstream`
needs the sidecar; a clamp-bounded `PerGroupRecompute` cell needs nothing.

## Spec delta (first)

`docs/specs/state.md` §"The degradation contract", step 2 — state that the required structure is
a function of the **cell**, not of its technique alone, and name the one cell-shaped
requirement: a `PerGroupRecompute` cell addressed by a key-addressed model edge requires the
**fingerprint sidecar** (its affected-key discovery is a group-grain sidecar diff), and where
the sidecar is unrealisable the cell downgrades to `DeleteInsert` — recorded as
`MaintenanceStateDowngraded`, never refused at execution. Add one sentence to §"Constraints &
Invariants" that no consumer may re-derive this requirement at run time.

## Tests (red first)

1. `smelt-logical` `maintenance_availability::resolution::key_addressed_per_group_cell_requires_the_sidecar`
   — `required_state_structure` over a `PerGroupRecompute` cell carrying an `UpstreamKeyed`
   `key_scope` is `Some(FingerprintSidecar)`.
2. `…::clamp_bounded_per_group_cell_requires_no_structure` — the same technique **without** a
   `key_scope` still requires nothing (guards against widening the rule to every recompute cell).
3. `…::key_addressed_cell_downgrades_to_delete_insert_without_the_sidecar` —
   `resolve_availability` with `StateAvailability::none()` sets `technique = DeleteInsert` and
   records `StateDowngrade { original: PerGroupRecompute, missing: FingerprintSidecar, .. }`.
   `recompute_equivalent`'s existing `key_scope.is_some() → PerGroupRecompute` rule must not
   produce a no-op downgrade here.
4. `…::downstream_grain_over_upstream_cell_requires_the_sidecar` — the second sidecar-backed
   discovery route is covered too; `EnrichmentKeyed` is not (it addresses `ColumnScopedMerge`,
   already `MergeLedger`-gated).
5. `…::key_addressed_cell_survives_when_the_sidecar_is_available` — under
   `StateAvailability::all()` the cell keeps `PerGroupRecompute` and carries no downgrade.
6. `smelt-runtime` `key_addressed_model_edge_lowering::key_addressed_edge_returns_none_under_honest_spark_availability`
   — `resolve_live_key_addressed_model_edge_cell` with
   `StateAvailability::resolve(Allowed, &realisable_state_structures(SparkSQL))` returns
   `Ok(None)` (the cell was downgraded away) rather than `Err`. Existing test 3
   (`key_addressed_edge_refuses_without_the_sidecar_capability`) **stays** and keeps passing: it
   passes `StateAvailability::all()` against a `false` capability flag, i.e. the inconsistent-
   inputs case, where the fail-loud bail is still correct. Retitle its doc comment to say so.

## Tasks

1. Land the `state.md` spec delta above.
2. Change `required_state_structure(technique: Technique)` to `required_state_structure(cell: &PlanCell)`
   in `availability/state_structure.rs`; keep the technique match as the base case and add the
   cell-shaped `PerGroupRecompute` + sidecar-backed `key_scope.discovery` arm, exhaustive over
   `KeyDiscovery` so a new route is a compile error. Document the two sources of truth
   (`realisable_state_structures` and `BackendCapabilities::supports_fingerprint_sidecar`) as
   required to agree.
3. Teach `recompute_equivalent` that a cell whose missing structure is the `FingerprintSidecar`
   downgrades to `DeleteInsert` (the `key_scope.is_some()` arm must not return the technique
   being downgraded away from). Keep the function pure and total.
4. Update `resolve_availability` and the handful of callers/tests of the old signature
   (`crates/smelt-logical/tests/maintenance_availability/{resolution,succession}.rs`).
5. Update the doc comment on `maintenance_driver/key_addressed/mod.rs`'s `!supports_fingerprint_sidecar`
   bail: it is now a defensive guard for inconsistent inputs, not the primary route.
6. `cargo test --workspace` and fix fallout — expect `MaintenanceStateDowngraded` text to start
   appearing in `smelt-db` maintenance-diagnostics and `smelt explain` snapshots for
   key-addressed models on non-DuckDB targets. A new diagnostic on an existing fixture is the
   intended behaviour; a changed technique on **DuckDB** would not be, and must be investigated.
7. Live: `bash scripts/dbx-auth.sh` (the OAuth token expires hourly — re-mint before starting),
   then `bash scripts/dbx-verify.sh`.
8. Live: run three consecutive windows continuing the existing frontier (coverage currently ends
   `2026-08-10` for all models except `gold.events_enriched`, pinned at `2026-08-07`; the fixture
   holds days through `2026-08-20`):

   | step | loader | `smelt run` window |
   |---|---|---|
   | W4 | `--date 2026-08-10` | `--event-time-start 2026-08-10 --event-time-end 2026-08-11` |
   | W5 | `--date 2026-08-11` | `--event-time-start 2026-08-11 --event-time-end 2026-08-12` |
   | W6 | `--date 2026-08-12` | `--event-time-start 2026-08-12 --event-time-end 2026-08-13` |

   Each must report **16 success / 0 failed / 0 skipped**. `gold.events_enriched`'s first clean
   window recomputes its region from its pinned coverage, so expect it to be the slow model in
   W4 and ordinary in W5/W6 — record the wall times in `free-edition-facts.md`.
9. Read back `intervals.json`: every model, `gold.events_enriched` included, must end at
   `2026-08-13`. Record whether the technique downgrade provoked any definition-delta /
   migration-approval prompt (unexpected; if it does, record it, do not fix it).
10. Write `phases/07b-summary.md`: the shipped fix, the three clean windows, wall times, and
    anything the runs newly surfaced (which goes to row 10's punch-list, not a fix here).

## Verification

- `bash .claude/scripts/verify-phase.sh` — green, no ratchet lowered.
- `cargo test -p smelt-logical --test maintenance_availability` — the five new logical tests.
- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering` — new test 6 green,
  existing test 3 still green.
- `cargo test -p smelt-cli --test maintenance_conformance` — the equivalence-invariant gate,
  since this phase changes which technique a cell runs.
- Live: three run reports under `examples/github_activity/.smelt/targets/databricks/reports/`,
  each 16/16, and `intervals.json` coverage ending `2026-08-13` for every model.

## Commit message

`outcome(databricks-dogfood-spine): phase 7b downgrades the sidecar-less key-addressed cell at plan derivation`
