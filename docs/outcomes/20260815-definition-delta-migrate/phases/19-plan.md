# Phase 19 plan — Mutation-cell reachability

## Objective

Make the horizon-clamped `PartitionLocal::Yes` corner reachable from a real workspace through the
production plan wrapper, by moving `UpstreamMutation` trigger derivation out of the inline loop in
`smelt-db` into a pure `smelt-logical` function and widening its rule to cover (a) **clocked**
explicitly-`mutable_snapshot` sources and (b) `AppendOnly` sources a column group is genuinely
value-sensitive to. Advances success criterion 15's first and third clauses; the "mutation genuinely
happened" clause is phase 19b.

## Spec delta (spec-first — the implement step makes this edit)

`docs/specs/incremental_models.md`, §"Per-cell admission" (after obligation 7, before
"**Interchangeability and choice.**"): add a short **"Which changed inputs get a mutation cell"**
paragraph stating the derivation rule as behaviour, timelessly —

- a source gets an `UpstreamMutation` cell iff it **explicitly declares** `mutation_profile:
  mutable_snapshot` (the fail-closed admission default alone never synthesises one — an
  undeclared source is not silently treated as mutable), **or** it is `append_only` and named in
  some column group's value-sensitivity set (a late append into an already-written region changes
  stored values, so that region is maintained, not left stale);
- the source's clock is not part of the rule. A clocked mutable source whose scan the locality
  proof cannot clamp to the output axis surfaces the ordinary `MaintenanceScanUnbounded` refusal,
  escapable by `allow_full_scan` / `scan_bounds.on_violation: warn` — the same loud path an
  unclocked one already takes, never a silently-dropped cell.

Then narrow §Known Divergences "Plan-consumer gaps" to the single remaining clause (dispatch cannot
distinguish a genuine mutation from re-derivation), retargeting the ref at `phases/19b-plan.md`.

## Tests (red-green)

`crates/smelt-logical/tests/maintenance_plan_admission.rs` (new pure `derive_triggers`):
1. `clocked_mutable_source_gets_a_mutation_trigger` — a source with `partition_col: Some(..)` in the
   explicitly-mutable set yields `Trigger::UpstreamMutation`.
2. `undeclared_mutable_source_gets_no_mutation_trigger` — `MutableSnapshot` facts alone, absent from
   the explicitly-mutable set, yield only `NewData` (regression guard on the opt-in rule).
3. `append_only_source_in_a_value_sensitive_group_gets_a_mutation_trigger` — an `AppendOnly` source
   named in a `ColumnGroup::mutation_sensitivity` yields `UpstreamMutation`.
4. `append_only_source_with_no_value_sensitivity_gets_no_mutation_trigger` — a pass-through
   append-only read yields only `NewData`.
5. `trigger_derivation_is_order_stable_and_deduplicated` — one trigger per source, deterministic
   order (the plan is pure data consumers compare).

`crates/smelt-db/tests/maintenance_diagnostics.rs` (production wrapper):
6. `daily_events_status_derives_a_status_mutation_cell_through_the_wrapper` — `examples/timeseries`'s
   `daily_events_status` derives a `{status}` `UpstreamMutation{raw.user_status}` cell with
   `PartitionLocal::Yes` and `Technique::ColumnScopedMerge`, with no refusals.
7. `clocked_mutable_source_with_no_derivable_clamp_refuses_scan_unbounded` — the same declaration
   over a model whose SQL carries no linking predicate refuses loudly (`Refusal::ScanUnbounded`)
   rather than dropping the cell.

`crates/smelt-runtime/tests/technique_lowering.rs`:
8. Rewrite `real_fixture_daily_events_status_would_admit_partition_local_yes_cell` as
   `real_fixture_daily_events_status_admits_partition_local_yes_cell` — it must obtain the plan from
   the production wrapper (`smelt_db::queries::maintenance`) and assert the same `{status}` cell,
   deleting the hand-built trigger list and the "Known production gap" comment block that justified
   it. Keep `yes_corner_matches_full_refresh_after_dimension_mutation` passing unchanged.

## Tasks

1. Make the spec edit above (spec-first), including the narrowed Known Divergences bullet.
2. Add `pub fn derive_triggers(...) -> Vec<Trigger>` to
   `crates/smelt-logical/src/maintenance/derive.rs` — inputs: `&[SourceFacts]`, `&[ColumnGroup]`,
   the explicitly-mutable source-name set, and the `added_columns` list; emits `NewData` per source,
   `UpstreamMutation` per the rule above, `Backfill`, and `ColumnAdded` when non-empty. Doc comment
   records the rule and why the clock is not part of it (replacing the two "deliberately NOT derived"
   paragraphs being deleted from `smelt-db`).
3. Write tests 1–5 red, then land the function.
4. Replace the inline trigger loop in
   `crates/smelt-db/src/queries/maintenance.rs::derive_model_maintenance_plan` with a call to it;
   no logic restated on the `smelt-db` side (maintenance-plan purity).
5. Write tests 6–7 red against the wrapper, confirm green after task 4.
6. Rewrite runtime test 8 onto the production path.
7. Run the example/LSP gates and triage any *new* diagnostic on `examples/timeseries` or
   `examples/broken`: a new `MaintenanceScanUnbounded` on `daily_events_status` means the fixture's
   clamp is not deriving and is a real red, not something to silence with `allow_full_scan`.
8. `docs-site/docs/reference/smelt-yml.md`: one sentence under `mutation_profile` that declaring
   `mutable_snapshot` derives a mutation cell whether or not the source is clocked, and one under
   `append_only` that an append-only source read by an aggregate gets one too.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test maintenance_plan_admission --test maintenance_coverage_matrix --test locality_projection`
- `cargo test -p smelt-db --test maintenance_diagnostics --test maintenance_model_upstream`
- `cargo test -p smelt-runtime --test technique_lowering --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance --test example_diagnostics`
- `cargo test -p smelt-lsp --test example_workspaces`

## Commit message

`feat(maintenance): derive UpstreamMutation triggers purely, covering clocked mutable and aggregate-sensitive append-only sources`
