# Phase 28b summary — pin the merged-group region-recompute rule

**Shipped:**
- `derive_mutation` (`crates/smelt-logical/src/maintenance/derive.rs`) now computes, per
  column group, the count of distinct sources in `mutation_sensitivity ∪
  membership_sensitivity` that are actually mutation-capable (present in `covered_by_mutation`,
  the same set `derive_triggers` already derives — no second guess at the predicate). A count
  ≥ 2 forces `(Corner::RecomputeRegion, Technique::DeleteInsert)`, same as the existing
  membership-sensitivity branch, never `ColumnScopedMerge`.
- New unit tests `crates/smelt-logical/tests/maintenance_merged_group.rs` (4 tests): merged
  group → region recompute; single-input control keeps the column merge; an append-only
  co-sensitive source doesn't count toward the merge; the pre-existing membership-sensitivity
  branch is unaffected.
- New end-to-end fixture `merged_group_fixture_plans_region_recompute` in
  `crates/smelt-logical/tests/maintenance_coverage_matrix.rs`, driven through the real
  `grouping::derive_column_groups` derivation (a two-mutable-dimension `LEFT JOIN` enrichment
  model), confirming the merged group lands on region recompute via the production path, not
  just hand-built `ModelInputs`.
- `docs/specs/incremental_models.md` §Known Divergences: removed "The merged-group
  region-recompute rule is unverified in the implementation".

**Decisions:**
- "Mutation-capable" is defined as "gets its own `UpstreamMutation` trigger" (read off
  `covered_by_mutation`), not "is declared `MutableSnapshot`" — an `AppendOnly` source with no
  value-sensitivity of its own, or one this model derives no mutation cell for, does not count.
  This matches `membership_sensitive`'s existing scoping and avoids over-triggering the guard
  for sources that never get a repair cell in the first place.

**For the next planner:**
- Structural finding worth recording: a genuinely non-membership-sensitive real SQL fixture for
  the merged-group guard (two mutable inputs blended in one payload column, via a JOIN, with
  neither becoming membership-sensitive) appears to be effectively unreachable — the same
  closure proof that would prune JOIN-derived membership sensitivity (`skeleton_source_closure`'s
  conjunct 1) requires the enrichment column NOT blend sources, which is exactly what a merged
  group's payload does. The e2e fixture therefore exercises a case where both provenance kinds
  (`mutation_sensitivity` and `membership_sensitivity`) hit the merged group together, same as
  the pre-existing `ex12_multi_input_merge_degenerates_to_recompute`; the *pure*-value-only path
  (my new guard's actual incremental behaviour change) is isolated only in the hand-built-inputs
  unit tests. Not a gap to close — just a note that the new guard's real-world reach may be
  narrower than the unit tests suggest, since a genuinely value-only two-mutable-input merge may
  not arise from ordinary JOIN SQL at all.
- No other follow-up work identified; success criterion 18's "Group-merge-provenance policy"
  bullet is now both decided and honoured.

**Gates:**
- `cargo test -p smelt-logical --test maintenance_merged_group` — pass (4/4)
- `cargo test -p smelt-logical --test maintenance_coverage_matrix --test maintenance_choice --test maintenance_plan_admission --test maintenance_tracer` — pass
- `cargo test -p smelt-runtime --test statement_parity --test tracer_maintenance` — pass
- `cargo test -p smelt-cli --test maintenance_conformance` — pass (74/74)
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `rg -n "merged-group region-recompute rule is unverified" docs/specs` — no hits
