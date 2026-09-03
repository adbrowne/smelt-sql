# Phase 28c summary — `change_feed` sources get an `UpstreamMutation` cell

## Shipped

- `MutationProfile::ChangeFeed` added to the plan layer
  (`crates/smelt-logical/src/maintenance/mod.rs`), with a single-owner `is_mutable()`
  predicate consulted everywhere a site previously restated `== MutableSnapshot`/
  `!= MutableSnapshot`.
- `derive_triggers` (`derive.rs`) gives a `ChangeFeed` source an unconditional
  `UpstreamMutation` trigger — declaration alone suffices, unlike `MutableSnapshot`'s
  `explicitly_mutable` gate.
- `derive_mutation`'s cell-technique choice is clamped: a `ChangeFeed` source's cell always
  takes `(Corner::RecomputeRegion, Technique::DeleteInsert)`, never `ColumnScopedMerge`.
- The fold-repair narrowing branch (`derive_new_data`) refuses fail-loud, naming the source,
  for a `ChangeFeed` posture instead of attempting `admit_per_group_recompute` — no
  fingerprint-sidecar discovery exists for a feed.
- `repair::discovery_posture` now returns `Option<RepairDiscoveryPosture>` (`None` for
  `ChangeFeed`); the one runtime caller (`maintenance_driver.rs`) and the `smelt explain`
  caller (`smelt-cli/src/explain.rs`) both bail/report loud rather than silently defaulting.
- `smelt-db`'s `source_facts` maps a declared `change_feed` source to
  `PlanMutationProfile::ChangeFeed` (previously fell through to the fail-closed
  `MutableSnapshot`).
- `execute.rs`'s landed-delta posture maps `ChangeFeed` to `SourceMutationPosture::MutableSnapshot`
  (whole-table, no interval representation — same as before, now via an explicit arm).
- `grouping.rs`'s three sensitivity/eligibility sites (value-sensitivity, membership-sensitivity,
  closure-pruning eligibility) treat `ChangeFeed` as mutable via `is_mutable()`.
- Spec: `docs/specs/incremental_models.md` §"Which changed inputs get a mutation cell" states the
  `change_feed` rule; the Known Divergences bullet is narrowed from "does not yet get a cell" to
  "always re-derives from the full input" (the still-open residue).

## Decisions

- `discovery_posture` returns `Option` rather than a silent fallback arm — matches the repo's
  fail-loud discipline; the `None` case is provably unreachable at runtime (derive-time refusal
  guarantees no `ChangeFeed` repair cell is ever admitted) but the caller still bails loud rather
  than trusting the invariant blindly.
- `explicitly_mutable` producers (smelt-db/lib.rs, queries/maintenance.rs, propagation.rs) needed
  no `ChangeFeed` entry — confirmed per task 5's audit: `derive_triggers`' unconditional-true arm
  for `ChangeFeed` makes that set irrelevant to it.
- `output_delta.rs`'s `to_maintenance_source_facts` fallback (`_ => MutableSnapshot`) was left
  untouched — not named in the plan's site list, and out of this phase's scope (a separate
  analysis-layer→plan-layer bridge, not the `derive`/`grouping`/`repair`/`execute` set the plan
  named).

## For the next planner

- The only remaining residue of the closed bullet is the honestly-open one already named in
  §Future Extensions: live fold machinery over a change feed's own delta shape (retractions,
  `delta_identity`) — blocked on the retraction/departure-rule prerequisite, tracked there.
- `output_delta.rs:676-677`'s `Some(MutationProfile::AppendOnly) => AppendOnly, _ => MutableSnapshot`
  fallback is still not `ChangeFeed`-aware; it wasn't in this phase's scope but is worth a look if
  a future phase touches output-delta shape derivation for change-feed sources.
- Phase 29 (key-grain frontmatter/CLI validation gaps) and phase 30 (statement_parity backbuild
  emitter extension) are next in the table.

## Gates

- `cargo test -p smelt-logical --test maintenance_change_feed --test maintenance_coverage_matrix --test maintenance_choice --test maintenance_plan_admission --test maintenance_plan_conformance` — pass
- `cargo test -p smelt-logical --lib source_shape_tests` — pass
- `cargo test -p smelt-db --test integration` — pass (366)
- `cargo test -p smelt-db --lib source_facts_maps_declared_change_feed` — pass
- `cargo test -p smelt-db --test maintenance_diagnostics change_feed_declared_source_derives_upstream_mutation_cell` — pass
- `cargo test -p smelt-runtime --test statement_parity --test tracer_maintenance` — pass
- `cargo test -p smelt-cli --test maintenance_conformance --test example_diagnostics` — pass
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full workspace test, example_diagnostics)
- `rg -n "do not yet get an \`UpstreamMutation\` cell" docs/specs` — no hits
