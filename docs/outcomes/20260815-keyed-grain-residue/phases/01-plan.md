# Phase 1 plan — `KeyedRetractableContribution`: classifier, diagnostic, fixture, test

## Objective

Advances success criterion 1. `KeyedRetractableContribution` is specified but produced by
nothing: no derivation site, no `Refusal`/`DiagnosticCode` variant, no test. This phase derives
it at the one site where both halves of its stated firing condition are already computed — a
retractable enrichment-join contribution *and* a repair family that cannot admit a per-group
recompute for the retraction — and surfaces it as an Error diagnostic naming the failing repair
obligation. No admission rule is invented: both halves reuse proofs that already exist
(`join_shape::join_contribution_monotone`, `repair::admit_per_group_recompute`).

## Where it derives (the seam)

`smelt-logical/src/maintenance/derive.rs`, the key-grain `NewData` per-source handler, in the
`Err(refusal)` arm of the existing `repair::admit_per_group_recompute` call (~line 1364). That
arm *is* "the repair family cannot admit a per-group recompute for the retraction". Adding the
join-contribution test in front of the existing refusal pushes is additive — the pre-existing
`NoAdmissibleTechnique` + `RepairKeysNotDiscoverable`/`RepairSliceUnbounded` refusals stay, so no
existing expectation changes. Firing is impossible on join spelling alone: the branch is only
reached after the faithful-fold posture obligation has already failed for this source.

## Spec delta

- `docs/specs/incremental_shapes.md` §Known Divergences — delete the bullet
  "**`KeyedRetractableContribution` has no implementation (Open Question)**".
- `docs/specs/diagnostics.md` line ~180 and `incremental_shapes.md` §Diagnostics table row —
  no wording change needed (both already state the target behaviour); verify no "not yet
  produced" annotation remains for this code anywhere under `docs/specs/`.
- `crates/smelt-db/src/queries/maintenance.rs` ~line 1391: the comment claiming the repair
  refusals are "not yet produced by any wired derivation" is stale (derive.rs line 1333 produces
  them) — correct it while touching the neighbouring match arms.

## Tests

Red-green, in this order:

1. `crates/smelt-logical/tests/repair_wiring.rs::retractable_enrichment_contribution_refuses_by_name`
   — a `grain: key` model summing a value off a joined `mutable_snapshot` dimension that declares
   no `unique_key` (fan-out unprovable) and whose repair admission fails: the derived plan carries
   `Refusal::KeyedRetractableContribution` naming the failing repair obligation.
2. `…::monotone_enrichment_contribution_emits_no_retractable_refusal` — same model shape but the
   dimension feeds `MAX(...)` (order/value-monotone, never decrementing): the plan carries the
   pre-existing refusals and *no* `KeyedRetractableContribution` (the "never on join spelling
   alone" guarantee).
3. `…::admitted_repair_emits_no_retractable_refusal` — a retractable contribution whose repair
   *does* admit (`Ok(AdmittedRepair)`): a repair cell, no `KeyedRetractableContribution`.
4. `crates/smelt-db/tests/maintenance_diagnostics.rs::keyed_retractable_contribution_is_an_error_diagnostic`
   — the fixture from test 1, loaded through `file_diagnostics()`, yields exactly one
   `DiagnosticCode::KeyedRetractableContribution` at Error severity, whose message names the
   failing repair obligation and steers to `refresh: materialized_view` or DAG composition.
5. `crates/smelt-logical/src/analysis/join_shape.rs::join_alias_for_source_*` — unit coverage for
   the alias resolver moved in from `smelt-runtime` (aliased join, unaliased join, no join →
   `None`).

## Tasks

1. Move `find_join_alias` from `smelt-runtime/src/maintenance_driver.rs` into
   `smelt-logical/src/analysis/join_shape.rs` as `pub fn join_alias_for_source(sql, source) ->
   Option<String>`; `dimension_join_contribution` calls it (behaviour unchanged, no duplicate).
2. Add `Refusal::KeyedRetractableContribution { source: String, columns: Vec<String>, why: String }`
   to `smelt-logical/src/maintenance/mod.rs` with a doc comment citing
   `incremental_shapes.md` §"Enrichment joins".
3. In `derive.rs`'s repair-`Err` arm: resolve the source's join alias; build a `JoinContext` from
   `SourceFacts::unique_key` (`with_composite_unique_key`); for each fold column the source feeds
   (`fold.add_columns`, filtered by the column group's `mutation_sensitivity` for this source)
   compose `fan_out` with `combiner_discriminants` via `join_contribution_monotone`. Push the new
   refusal only for columns whose verdict is `Refused`, with `why` = the contribution reason plus
   the failing repair obligation (`KeysNotDiscoverable`/`SliceUnbounded`) verbatim. No join
   against the source, or a `Monotone` verdict → push nothing new.
4. Map it through: `MaintenanceRefusal::KeyedRetractableContribution` in
   `smelt-db/src/queries/maintenance.rs` (replacing the `None` idiom for this code only), a
   `DiagnosticCode::KeyedRetractableContribution` variant in `diagnostics_types.rs`, and an Error
   arm in `smelt-db/src/lib.rs`'s refusal match whose message ends with the spec's steer
   ("use `refresh: materialized_view`, or compose the enrichment as a separate model").
5. Land the fixture: a `grain: key` model + `mutable_snapshot` source in the test harness the
   chosen test files already use (no `examples/broken/` addition — the example workspaces are
   gated clean/expected-set and this is a maintenance-plan refusal, not a parse error).
6. Apply the spec delta and the stale-comment correction above.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test repair_wiring`
- `cargo test -p smelt-db --test maintenance_diagnostics`
- `cargo test -p smelt-runtime --test statement_parity` and `--test technique_lowering`
  (the `find_join_alias` move's blast radius)
- `cargo test -p smelt-cli --test maintenance_conformance` (no new refusal on admitted recipes)
- `rg -n 'KeyedRetractableContribution' docs/specs/` shows no "no implementation"/"Open Question"
  wording left.

## Commit message

`feat(incremental): KeyedRetractableContribution is derived, diagnosed, and tested`
