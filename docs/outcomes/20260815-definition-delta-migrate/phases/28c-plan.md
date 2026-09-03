# Phase 28c — `change_feed` sources get an `UpstreamMutation` cell

## Objective

Give the plan layer a `MutationProfile::ChangeFeed` kind so a source declaring
`mutation_profile: change_feed` derives an `UpstreamMutation` cell like every other
mutation-sensitive posture, admitted conservatively as full-input re-derivation. Advances
success criterion 18 (every still-live Known Divergences bullet is closed or narrowed): the
"`change_feed` sources do not yet get an `UpstreamMutation` cell" bullet narrows to the
residue — no live fold over a feed's own delta shape.

## Spec delta (do first)

`docs/specs/incremental_models.md`:
- §"Which changed inputs get a mutation cell" (~L973): extend the rule — a source that
  explicitly declares `mutation_profile: change_feed` also gets an `UpstreamMutation` cell.
  Unlike `mutable_snapshot` the declaration alone suffices (a change feed can only arise from
  an explicit declaration, so there is no fail-closed default to guard against), and the
  resulting cell is admitted as full-input re-derivation over the source's current contents —
  the feed's own delta rows are not read.
- §Known Divergences (~L2100): replace the bullet with the narrowed residue — a `change_feed`
  source's cell re-derives from the full input; live fold machinery over the feed's delta shape
  (retractions, `delta_identity`) remains §Future Extensions, blocked on the retention point.
- §Future Extensions: confirm/point the change-feed fold entry at the narrowed bullet.

## Tests

`crates/smelt-logical/tests/maintenance_change_feed.rs` (new):
- `change_feed_source_derives_upstream_mutation_trigger` — `derive_triggers` emits
  `Trigger::UpstreamMutation` for a `ChangeFeed` source with no entry in `explicitly_mutable`.
- `change_feed_cell_takes_full_input_rederivation` — the derived cell for a change-feed-sensitive
  group is `(Corner::RecomputeRegion, Technique::DeleteInsert)` with no partition-local clamp,
  never `ColumnScopedMerge` / a sidecar-diff repair cell.
- `change_feed_group_is_value_sensitive_like_mutable_snapshot` — `derive_column_groups` records
  a change-feed leaf in `mutation_sensitivity` (non-aggregate read included), same as
  `MutableSnapshot`.
- `change_feed_repair_cell_is_refused_not_silently_admitted` — a shape that would otherwise
  admit `Technique::PerGroupRecompute` over a change-feed source refuses fail-loud naming the
  source, rather than falling through to a sidecar diff that does not exist.
- `change_feed_source_shape_maps_to_change_feed_delta_profile` — `source_shape` maps the plan
  kind onto `DeltaMutationProfile::ChangeFeed`, closing the "no `ChangeFeed` in the plan layer"
  1:1 gap in its doc comment.

`crates/smelt-db/tests/` (existing maintenance test file, add):
- `source_facts_maps_declared_change_feed` — `source_facts` yields
  `PlanMutationProfile::ChangeFeed` for a declared change feed, while undeclared/`mutable_snapshot`
  keep failing closed to `MutableSnapshot`.

End-to-end:
- `examples/source_mutation_profile_declared` (its `raw_events` source is already
  `mutation_profile: change_feed`): assert via the production maintenance-plan path that the
  model over it now carries an `UpstreamMutation` cell — extend
  `crates/smelt-logical/tests/maintenance_coverage_matrix.rs` or the nearest existing
  workspace-driven maintenance test rather than adding a new harness.

## Tasks

1. Land the spec delta above.
2. Add `ChangeFeed` to `MutationProfile` in `crates/smelt-logical/src/maintenance/mod.rs`; add a
   single-owner predicate (e.g. `fn is_mutable(self) -> bool`) so "is this posture mutable?" is
   answered in one place instead of restated at each match site.
3. Fix every resulting non-exhaustive match, deciding each arm explicitly (no `_` catch-alls in
   plan-layer code):
   - `smelt-db/src/queries/maintenance.rs` `source_facts` — `Some(ChangeFeed) => ChangeFeed`;
     keep the fail-closed `_ => MutableSnapshot` for undeclared/`Mutable`.
   - `derive.rs` `derive_triggers` — `ChangeFeed => true` (declaration is the explicit statement).
   - `derive.rs` `source_shape` — map to `DeltaMutationProfile::ChangeFeed`; update the doc
     comment that currently records the gap.
   - `derive.rs` faithful-fold posture (~L1287) — `ChangeFeed => DeltaMutationProfile::ChangeFeed`.
   - `grouping.rs` value-sensitivity (~L373), membership-sensitivity (~L721), closure-pruning
     eligibility (~L832) — treat `ChangeFeed` as mutable via the new predicate.
   - `repair.rs` `discovery_posture` — no sidecar exists for a feed; either make the arm
     unreachable by refusing the repair cell upstream in `derive`, or return a refusal. Whichever
     route, the refusal must be loud and named, never a silent fallback.
   - `smelt-runtime/src/execute.rs` (~L4383) landed-delta posture — `ChangeFeed =>
     SourceMutationPosture::MutableSnapshot` (whole-table, no interval representation).
4. Clamp the change-feed mutation cell to full-input re-derivation in `derive_mutation` (same
   forcing shape phase 28b's merged-group guard uses), so no column-scoped or fold realisation
   is reachable for a posture whose delta machinery does not exist.
5. Audit the `explicitly_mutable` producers (`smelt-db/src/lib.rs` L1945,
   `smelt-db/src/queries/maintenance.rs` L1220, `smelt-runtime/src/propagation.rs` L729) —
   confirm `ChangeFeed` needs no entry there given task 3's unconditional derivation, and record
   the finding in the summary if it does.
6. Narrow the Known Divergences bullet (spec delta) and check no other spec/doc-site page still
   claims a change feed gets no cell (`rg -n "change_feed" docs/specs docs-site/docs`).

## Verification

- `cargo test -p smelt-logical --test maintenance_change_feed --test maintenance_coverage_matrix
  --test maintenance_choice --test maintenance_plan_admission --test maintenance_plan_conformance`
- `cargo test -p smelt-db --test integration` (source_facts mapping) and
  `cargo test -p smelt-runtime --test statement_parity --test tracer_maintenance`
- `cargo test -p smelt-cli --test maintenance_conformance --test example_diagnostics`
- `bash .claude/scripts/verify-phase.sh`
- `rg -n "do not yet get an \`UpstreamMutation\` cell" docs/specs` — no hits

## Commit message

`feat(maintenance): derive an UpstreamMutation cell for change_feed sources`
