# Plan: Membership sensitivity — honest dimension-mutation repair for keyed enriched models

**Date**: 2026-08-08
**Spec**: [`docs/specs/incremental_models.md`](../specs/incremental_models.md) §"The plan matrix" (sensitivity kinds), §Design "Membership sensitivity is derived, never inferred from collapse"; [`docs/specs/model_properties.md`](../specs/model_properties.md) §"Per-column mutation-sensitivity / column provenance" (membership-sensitivity paragraph)
**Spec diff**: uncommitted working tree (sensitivity kinds: value vs membership; membership-sensitive groups require the recompute family; Known Divergences entry rewritten)
**Tracking PR / branch**: `spec-redraft-incremental-models`
**Docs**: code-only  <!-- derivation/technique internals; no new user-declared surface. Spec edits are the diff above. -->

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file, then the spec sections named above — they are the correctness oracle. Do not re-open settled spec decisions.
2. Confirm you are on branch `spec-redraft-incremental-models`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table and start there. If every phase is `done`, run "Verification" and stop.

**For each phase:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**
- The reviewer surfaces the same material finding across two implementer passes.
- TDD tests cannot be made green without violating a spec rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` first).
- A pre-existing failure unrelated to the plan appears.

**Conventions every phase:**
- Red-green TDD; failing test before implementation.
- The equivalence invariant is the oracle: every admission widening or technique change must be proven by a real-DuckDB conformance case (`maintenance_conformance` harness), not only unit-asserted.
- Verification gate: `bash .claude/scripts/verify-phase.sh`, foreground, one call.
- Atomic per-phase commits with the phase's `Commit.` line verbatim; never amend a pushed phase.
- Don't widen scope into later phases.
- Timeless-oracle rule: phase vocabulary stays in this file.

---

## Context

Established empirically (2026-08-08, steering session for `docs/plans/20260808-substrate-unification.md`): swapping `maintenance/grouping.rs` onto the gated column-ref collector removes the accidentally-admitted `UpstreamMutation(dim)` cell for the keyed-enriched shape and derives a zero-refusal plan that dispatches `cumulative_aggregate` — leaving a mutable inner-join dimension's mutations entirely unmaintained (no cell, no refusal). The spec now names the missing concept — membership sensitivity — and requires membership-sensitive groups to take the recompute family. This plan lands the derivation, the technique assignment, the runtime dispatch, and a conformance extension that finally exercises genuine dimension mutations. Full context map: the steering session's exploration report is summarized in `docs/TODO.md` §"`maintenance::grouping`'s column-ref collector keeps a known under-collection bug".

## Scope

### In scope (spec coverage)
- `model_properties.md` membership-sensitivity derivation: row-admission reads of mutable sources contribute row-scoped sensitivity.
- `incremental_models.md` §"The plan matrix" sensitivity kinds: membership-sensitive cells assign the recompute family (delete+insert, change-suppressed where comparable), never column-scoped merge.
- Collector swap: `grouping.rs` onto gated `collect_column_refs`; `collect_column_refs_ungated` deleted.
- Conformance: the keyed-enriched fixtures rewritten to the honest plan; generator extended to mutate dimensions for real.
- Known Divergences entry deleted once landed.

### Explicitly deferred
- **Delta-restricted membership repair** (recomputing only keys whose join outcome could have changed, via a change feed or derived fingerprint diff) — v1 is whole-model recompute with change-suppressed writes; delta restriction is a cost optimization on an already-sound cell, `docs/research/20260715-conditional-maintenance-without-cdf.md` is the design home.
- **Monotone-join admission relaxation** (`docs/research/20260704-monotone-join-maintenance.md`) — proving a dimension read monotone to avoid membership cells entirely; orthogonal widening.
- **Outer-join membership semantics** (LEFT JOIN null-extension rows changing on dim mutation) — derive fail-closed (membership-sensitive) for now; refined footprints later.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1     | done     | 3d83305f | 2026-08-08 |
| 2     | done     | 1c208806 | 2026-08-08 |
| 3     | done     | 8e3c0877 + b5f42fef | 2026-08-08 |
| 4     | done     | (this commit) | 2026-08-08 |

---

### Phase 1: Membership-sensitivity derivation + collector swap

**Goal.** `derive_column_groups` derives membership sensitivity from row-admission reads (join predicates over mutable sources) as its own kind, alongside value sensitivity from the (now gated) select-item collector; the ungated collector is deleted.

**Pre-conditions.** None (substrate-unification plan complete).

**TDD tests to write first.**
- `crates/smelt-logical/tests/maintenance_grouping.rs::join_only_mutable_dim_is_membership_sensitive` — the keyed-enriched shape (`SELECT f.id, COUNT(f.val) … FROM fact f JOIN dim ON f.id = dim.id GROUP BY f.id`, dim `MutableSnapshot`): every column group carries membership sensitivity to `dim`, value sensitivity only to `fact`; no degenerate collapse. RED today (collapse admits dim by accident; post-swap-alone it would vanish).
- `crates/smelt-logical/tests/maintenance_grouping.rs::append_only_join_partner_contributes_no_membership` — same shape with dim `AppendOnly` over an inner join whose admitted rows cannot be un-admitted by appends… careful: an inner-join append CAN admit previously-unmatched fact rows retroactively. Assert what the spec says: membership sensitivity derives from *mutable* sources' admission reads; an `AppendOnly` partner's retroactive-admission hazard is the pre-existing enrichment-admission question, out of scope — assert the derivation does not change verdicts for the existing `MutableEnrichedRecipe`/`KeyedRecipe` fixtures (characterization guard).
- `crates/smelt-logical/tests/maintenance_grouping.rs::function_wrapped_ref_collects_arguments` — `SUM(a.x)` contributes `a.x` value sensitivity, no bogus `SUM` column (the collector-swap red test).
- Rewrite `crates/smelt-logical/tests/expr_util.rs::function_wrapped_source_column_still_collapses_under_the_preserved_bug` to its post-fix twin (non-degenerate, correctly-sensitive group).
- Plan-level: `crates/smelt-logical/tests/maintenance_tracer.rs` (or sibling) — the keyed-enriched shape's derived plan has an `UpstreamMutation(dim)` cell marked membership-sensitive with a recompute-family technique, zero refusals, and NO `ColumnScopedMerge` for that cell.

**Implementation shape.** `ColumnGroup` (maintenance/mod.rs) gains sensitivity kind — e.g. `mutation_sensitivity: BTreeSet<String>` splits into value + membership sets (or entries typed by kind). `derive_column_groups` scans join predicates (`ON` conjuncts, via the shared `expr_util` splitter/collector) for reads of mutable sources and attaches membership sensitivity to all groups of rows those joins admit. `derive_mutation` (derive.rs) assigns membership-sensitive cells `Corner::RecomputeRegion` + `Technique::DeleteInsert` with the change-suppressed write variant where comparable; `ColumnScopedMerge` assignment requires pure value sensitivity. `grouping.rs` imports the gated collector; `collect_column_refs_ungated` deleted from `expr_util.rs`.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/{grouping.rs,mod.rs,derive.rs}`
- `crates/smelt-logical/src/analysis/expr_util.rs` — delete ungated collector + its doc paragraph
- `crates/smelt-logical/tests/{maintenance_grouping.rs,maintenance_tracer.rs,expr_util.rs}`
- Mechanical compile-fix fallout in direct `ColumnGroup` consumers (smelt-db queries, explain, runtime lowering) — signature only, no behaviour change outside the keyed-enriched shape

**Review checklist**:
- [ ] Membership sensitivity derived from join-predicate reads via shared expr_util helpers (no new ad hoc scan — walk-coverage gate stays green)
- [ ] Value-sensitivity verdicts unchanged for all existing fixtures except the keyed-enriched/function-wrapped shapes (characterization guard passes)
- [ ] `collect_column_refs_ungated` gone (grep)
- [ ] Membership cells cannot receive `ColumnScopedMerge` (negative test)

**Commit.** `feat(logical): derive membership sensitivity; gated collector in column grouping`

---

### Phase 2: Keyed-path dispatch for membership recompute

**Goal.** The runtime dispatches a keyed model's membership-sensitive `UpstreamMutation` cell through the delete+insert recompute (staged + change-suppressed), replacing the column-scoped-merge keyed dispatch for that shape.

**Pre-conditions.** Phase 1.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/technique_lowering.rs` — rewrite `mod keyed_column_scoped_merge_e2e` to the membership shape: the keyed-enriched model's dim cell lowers to delete+insert recompute with suppression; a genuine dim change (changed attribute affecting join membership, e.g. dim row deleted) produces correct post-repair state; an unchanged redelivery produces zero writes (suppression).
- `crates/smelt-runtime/tests/statement_parity.rs` — the executed membership-recompute statements are byte-identical to the maintenance emitters' output (extend the DeleteInsert family leg to the keyed membership path if not already covered).

**Implementation shape.** `maintenance_driver.rs` (and the keyed/cumulative run loop where it consults the plan) routes membership-marked cells to the existing DeleteInsert resolution path with the staged-candidate/suppressed variant; strategy ledger label for the technique (e.g. `delete_insert_suppressed` — pick the existing naming convention from `resolve_write_variant`). No new emitters — the delete+insert and staged-candidate emitters exist; this is dispatch wiring.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/{maintenance_driver.rs,cumulative.rs,execute.rs,diagnostics.rs}`
- `crates/smelt-runtime/tests/{technique_lowering.rs,statement_parity.rs}`

**Review checklist**:
- [ ] Emitted statements come from the single-owner emitters (parity leg green)
- [ ] Suppression verified by a zero-write redelivery case
- [ ] A genuine membership change (dim row add AND dim row delete) repairs to full-refresh state

**Commit.** `feat(runtime): dispatch keyed membership cells through suppressed delete+insert recompute`

---

### Phase 3: Conformance — genuine dimension mutations under the oracle

**Goal.** The generative conformance gate exercises real dimension mutations and proves equivalence; the two keyed-enriched fixtures assert the honest plan.

**Pre-conditions.** Phases 1–2.

**TDD tests to write first.**
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs::keyed_enriched_recipe_admits_membership_recompute` (rewrite of `…_admits_suppressed_column_scoped_merge`) — the derived plan: membership-marked `UpstreamMutation(dim)` cell, recompute technique, zero Error diagnostics.
- `…::keyed_enriched_pool_upholds_equivalence_under_dim_mutation` (rewrite of `…_zero_write_redelivery`) — generated `KeyedSchedule`s extended so dimension windows can genuinely mutate (add a dim row matching existing fact rows; change a joined attribute; delete a dim row), driven through real `execute_project`, multiset-equal to the full-refresh oracle after every window; retain a zero-change redelivery window asserting suppression (zero writes).
- Generator: extend the keyed schedule/recipe machinery (`maintenance_conformance` harness + `smelt-maintenance-testkit` if that's where pools live) with dim-mutation windows, deterministic-seeded.

**Implementation shape.** Mostly test/harness work. If the oracle exposes a genuine repair bug, fix it in `smelt-logical`/`smelt-runtime` under this phase's review (that is the point of the gate) — but a fix that changes the derivation rules of Phase 1 re-opens Phase 1's review checklist; flag it.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/tests/maintenance_conformance/` (gate + harness), `crates/smelt-maintenance-testkit/`
- Bug-fix fallout in `smelt-logical`/`smelt-runtime` if the oracle catches a repair defect (flagged in the report)

**Review checklist**:
- [ ] Dim-mutation windows include add, change, and delete cases
- [ ] Equivalence asserted after every window, not only at the end
- [ ] Suppression still exercised (zero-write redelivery window)
- [ ] Seeds deterministic; failure output actionable

**Commit.** `test(conformance): keyed enriched models prove equivalence under genuine dimension mutations`

---

### Phase 4: Close-out — spec, TODO, deferred entries

**Goal.** Delete the Known Divergences entry; close the TODO and prior-plan deferred entries; final verification sweep.

**Pre-conditions.** Phases 1–3.

**TDD tests to write first.** None (docs phase); the sweep is the plan's Verification section run in full.

**Implementation shape.** Delete the `incremental_models.md` Known Divergences entry "Membership sensitivity is not yet derived…" (the body text is already timeless). Close `docs/TODO.md` §"`maintenance::grouping`'s column-ref collector…" and the substrate-unification plan's Phase-2 deferred entry (mark resolved with a pointer here — that file's Deferred section is append-only, so append the resolution note). Check `model_properties.md`'s "partial" classifier table row for mutation-sensitivity and update its status honestly.

**Critical files (allowed to touch in this phase).**
- `docs/specs/{incremental_models.md,model_properties.md}`, `docs/TODO.md`, `docs/plans/20260808-substrate-unification.md` (Deferred append), this plan's Progress table

**Review checklist**:
- [ ] No stale reference to the ungated collector or the accidental-collapse admission anywhere in specs (`rg -n "ungated|proven-by-accident|suppressed no-op" docs/specs/`)
- [ ] Spec edits timeless

**Commit.** `docs(spec): membership sensitivity landed — divergence entry closed`

---

## Deferred during implementation

(Append-only.)

- **Phase 1 (2026-08-08):** the derivation swap flips the `daily_events_enriched`
  fixture (and `MutableEnrichedRecipe`, `examples/timeseries/models/
  daily_events_status.sql`) from `ColumnScopedMerge` to `DeleteInsert` for its
  dimension-mutation cell — the intended effect, since `raw.users` is read in
  the enrichment JOIN's `ON` predicate and is now correctly membership-
  sensitive. This leaves the following tests red, all in the SAME shape
  family, all expected, all left for Phase 2 (runtime dispatch) / Phase 3
  (conformance rewrite) to resolve — none is a regression outside this
  family:
  - `smelt-runtime --test statement_parity::column_scoped_merge_statements_come_from_the_emitter`
  - `smelt-runtime --test technique_lowering` (`keyed_column_scoped_merge_e2e::*`,
    `column_scoped_merge_e2e::column_scoped_merge_dispatches_through_execute_project`,
    `real_fixture_examples_timeseries_admits_column_scoped_merge_cell`,
    `real_fixture_daily_events_status_would_admit_partition_local_yes_cell`)
  - `smelt-cli --test maintenance_conformance` (`gate::keyed_enriched_recipe_admits_suppressed_column_scoped_merge`,
    `gate::keyed_enriched_pool_upholds_equivalence_with_zero_write_redelivery`,
    `probes::dimension_mutation_touches_only_sensitive_groups`)
  - `smelt-cli --test bakeoff` (`pin_mutates_no_files`, `pin_emits_parseable_cells_entry`,
    `bakeoff_reports_measured_cost_per_admissible_technique`, `bakeoff_drops_scratch_unless_keep`)
    — the cell now has only one admissible technique, so there is nothing to
    measure/pin among alternatives
  - `smelt-cli --test bakeoff_seam` (`request_override_subject_to_admission`,
    `empty_overrides_change_nothing`, `request_override_forces_each_admissible_technique`)
  - `smelt-cli --test explain_model` (`explain_prints_no_recording_for_a_whole_row_identity_conditional_cell`,
    `explain_prints_observed_delta_recording_status_for_a_conditional_cell`)
  - `smelt-cli --test explain_show_sql::show_sql_statements_unaffected_by_observed_delta_report_rows`
  - `smelt-cli --test maintenance_pins` (`inadmissible_pin_fails_loud`, `prefer_is_soft_and_never_refuses`)

  Two test-only fixes were made in Phase 1 itself where the fallout was a
  pure diagnostic-count assertion, not a technique-choice assertion:
  `smelt-db/tests/maintenance_diagnostics.rs::unbounded_scan_refuses_by_default`
  and `smelt-cli/tests/example_diagnostics.rs::broken_workspace_maintenance_scan_unbounded`
  now expect 2 `MaintenanceScanUnbounded` diagnostics (one per membership-
  sensitive payload group) instead of 1 — both fixtures join a mutable,
  unclocked source only in the `ON` predicate, so both of the model's
  payload groups are now correctly membership-sensitive and each refuses
  independently.

- **Phase 2 (2026-08-08):** wired the runtime dispatch. `crates/smelt-runtime/
  src/maintenance_driver.rs` gained `resolve_live_membership_recompute_cell`
  (mirrors `resolve_live_column_scoped_cell` but filters on
  `Technique::DeleteInsert` over a proven `RowIdentity::Key(_)`, never
  `WholeRow` — `emit_staged_candidate_conditional` panics on an empty key)
  and `execute_staged_membership_recompute` (dispatches through
  `smelt_logical::maintenance::emit::emit_staged_candidate_conditional`,
  never a new emitter). `crates/smelt-runtime/src/execute.rs`'s
  `plan_is_keyed` branch now consults both resolvers side by side and
  reports the new dispatch's strategy label as `"delete_insert_suppressed"`
  (the plan's own suggested convention-consistent label — no existing label
  named this exact shape; `"column_scoped_merge"`/`"cumulative_aggregate"`
  are the sibling hand-picked snake_case strings this mirrors).

  **ColumnScopedMerge reachability verdict.** No fixture in this workspace
  reaches `ColumnScopedMerge` anymore. Both real fixtures that used to
  (`examples/timeseries/models/daily_events_enriched.sql`'s `{user_name}`
  cell, `daily_events_status.sql`'s `{status}` cell) read their mutable
  dimension in the join's own `ON` predicate — a row-admission read — so
  Phase 1's derivation correctly makes them membership-sensitive
  (`Technique::DeleteInsert`), and there is no currently-shipped shape where
  a mutable, row-admission-joined dimension is ALSO read in a select item
  with *only* value sensitivity (never membership) toward the same source.
  `statement_parity.rs::column_scoped_merge_statements_come_from_the_emitter`
  is rewritten to drive `execute_column_scoped_merge_full` directly against a
  `RecordingBackend` (the same synthetic-input pattern its
  `suppressed_column_scoped_merge_statements_come_from_the_emitter` sibling
  already used) rather than through a real fixture. Tracked here as a real
  reachability gap for Phase 4's spec/Known-Divergences pass, not silently
  worked around.

  **`emit_staged_candidate_conditional`'s departed-row limitation (inherited,
  not introduced).** That emitter's `DELETE` only ever removes a row MATCHED
  to a staged candidate row (`table.key = staged.key AND changed`) — a row
  whose key is entirely ABSENT from the recomputed candidate (a genuinely
  *departed* key: e.g. the dimension row a fact joined on was itself
  deleted, so the fact no longer appears in the model's own recompute at
  all) is never matched and is left stored, stale, forever. This is the
  SAME "region-scoped, absent = out of scope" semantics
  `staged_candidate_conditional_statements_come_from_the_emitter`
  (`statement_parity.rs`) already documents and tests ("user 3 … must be
  left untouched entirely"). `resolve_live_membership_recompute_cell`'s own
  doc comment records this; `crates/smelt-logical/src/maintenance/emit.rs`
  is outside Phase 2's critical files, so it is not fixed here. The new
  `technique_lowering.rs::keyed_membership_recompute_e2e::
  genuine_membership_change_repairs_to_full_refresh_state` test proves the
  dispatch correctly repairs an add-admission (a dim row added that matches
  EXISTING staged facts) and a delete-with-no-admitted-facts no-op, but
  deliberately does NOT exercise deleting a dim row that has currently-
  admitted facts (a genuine departure) — that scenario is a known-unsound
  repair under the current emitter and is tracked for Phase 3/4 alongside
  the conformance rewrite (`docs/specs/incremental_models.md` §Known
  Divergences already tracks reachability/soundness gaps in this family).

  **`MaintenancePlan::cell_for`'s first-match ambiguity (discovered, not
  introduced).** Membership sensitivity is a row-admission property of the
  WHOLE join, so every column group a membership-sensitive join admits (not
  only the group whose select item reads the mutable source) now carries
  its own `UpstreamMutation` cell for the SAME trigger —
  `daily_events_enriched.sql`'s fixture derives BOTH a `{user_name}` cell
  AND an `{event_id, event_type, user_id}` cell for `UpstreamMutation {
  raw.users }`. `MaintenancePlan::cell_for` (`crates/smelt-logical/src/
  maintenance/mod.rs`) returns only the FIRST matching cell for a trigger —
  safe when a trigger has one admitted cell, not when several groups share
  it. Both `resolve_live_column_scoped_cell` and `resolve_live_membership_
  recompute_cell` call `cell_for`, so in principle a keyed model with
  MULTIPLE membership-sensitive groups on the same trigger could have this
  phase's dispatch see only one group's `compared_columns`, under-covering
  the staged candidate's change-comparison set for the other group's
  columns. No currently-shipped fixture exercises this (the real fixtures
  either fall through to the always-correct region-recompute default
  regardless of which cell `cell_for` picks, or — `user_lifetime_status` —
  only ever derive a single group for the trigger). `crates/smelt-logical/
  src/maintenance/mod.rs` is outside Phase 2's critical files; the two
  `real_fixture_*` unit tests in `technique_lowering.rs` were adapted to
  search `plan.cells` for the specific group under test rather than rely on
  `cell_for`. Flagged here as a genuine finding for a follow-up, not
  silently worked around.

  **Full failure survey after Phase 2**, `cargo test --workspace
  --no-fail-fast`: identical 15-test failure set to Phase 1's own survey
  above (`bakeoff` ×4, `bakeoff_seam` ×3, `explain_model` ×2,
  `explain_show_sql` ×1, `maintenance_conformance` ×3, `maintenance_pins`
  ×2) — no new failures, no flips. `smelt-runtime`, `smelt-logical`,
  `smelt-db` all fully green.

- **Phase 1 reviewer follow-up (2026-08-08):** the first pass only scanned
  `JOIN` `ON` predicates; a reviewer pass found the same silent-hole shape
  relocated to `WHERE`/`HAVING` — `WHERE o.user_id IN (SELECT user_id FROM
  smelt.sources.<mutable>)` (a semi-join admission read, named explicitly
  in `model_properties.md`'s membership paragraph) derived zero sensitivity
  of either kind and zero collapse. Fixed in the same
  `membership_sensitivity_sources` (`crates/smelt-logical/src/maintenance/
  grouping.rs`): top-level `WHERE`/`HAVING` conjuncts are now scanned the
  same way `ON` conjuncts are (shared `resolve_conjunct` closure); a direct
  column read of a `MutableSnapshot` source contributes membership
  sensitivity, an `AppendOnly` read contributes nothing, and ANY subquery
  inside a `WHERE`/`HAVING` conjunct (`IN`/`EXISTS`/scalar — detected via a
  `SELECT_STMT` descendant scan) fails closed to the whole-model collapse
  rather than being resolved into. Three new tests added to
  `maintenance_grouping.rs`: `where_in_subquery_over_mutable_source_
  collapses_fail_closed`, `direct_where_read_of_mutable_dim_is_membership_
  sensitive`, `where_filter_on_append_only_fact_contributes_no_membership`.
  Full failure survey re-run after the fix: identical failure set to the
  entry above, no new flips — the WHERE/HAVING scan only ever *adds*
  sensitivity (or collapses) on top of the existing ON-predicate scan, and
  none of the already-catalogued fixtures has a WHERE/HAVING conjunct
  reading a mutable source that the ON-predicate scan hadn't already
  caught.

- **Phase 3 (2026-08-08):** the departed-key repair bug the reviewer
  checklist flagged as expected in this phase was fixed properly, not
  worked around. `crates/smelt-logical/src/maintenance/emit.rs` gained
  `emit_staged_candidate_conditional_recompute` — a SIBLING to
  `emit_staged_candidate_conditional`, not a modification of it: that
  function's own "absence from the candidate = out of this run's touched
  region, leave untouched" contract is correct and load-bearing for its own
  region/window-scoped callers (`crates/smelt-runtime/tests/
  statement_parity.rs::staged_candidate_conditional_statements_come_from_the_emitter`'s
  "user 3 … must be left untouched entirely";
  `crates/smelt-cli/tests/maintenance_conformance/gate.rs::
  keyed_pool_t1_t2_and_full_refresh_agree_at_fixed_s`'s "device 3 (absent
  from the delta) must never be touched" — both still pass unmodified). The
  new emitter's `candidate_select` is always a FULL, unwindowed recompute
  (the single production caller, `crates/smelt-runtime/src/
  maintenance_driver.rs::execute_staged_membership_recompute`, only ever
  passes the model's own full re-derivation), so absence there genuinely
  means departure, and the extra anti-join `DELETE FROM <table> WHERE NOT
  EXISTS (SELECT 1 FROM <staged> WHERE <key join>)` removes it — a no-op
  when nothing has departed, keeping the change-suppressed zero-write
  contract intact (`crates/smelt-cli/tests/maintenance_conformance/
  gate.rs::keyed_enriched_pool_upholds_equivalence_under_dim_mutation`'s
  redelivery window: full table snapshot byte-identical before/after).

  **A new, real reachability gap surfaced while rewriting the 12 CLI-surface
  fixtures** (bakeoff ×4, bakeoff_seam ×3, explain_model ×2,
  explain_show_sql ×1, maintenance_pins ×2), confirmed empirically, not
  merely reasoned about: `daily_events_enriched`'s (and every JOIN-based
  fact+mutable-dimension enrichment fixture's) `UpstreamMutation` cell
  family is now uniformly membership-sensitive `Technique::DeleteInsert` —
  `Technique::ColumnScopedMerge` is unreachable from ANY currently-shipped
  SQL shape in this workspace (confirmed via
  `membership_sensitivity_sources`, `crates/smelt-logical/src/maintenance/
  grouping.rs`: ANY `JOIN`'s `ON` predicate — inner or left — reading a
  `MutableSnapshot` source makes EVERY column group of that `SELECT`
  membership-sensitive, not only the columns the dimension itself
  contributes; membership sensitivity is row-scoped, not per-column). Two
  concrete, previously-untested consequences:
  - `smelt bakeoff`'s measured/`--pin` code path (`run_bakeoff`'s branch
    past the `candidates.is_empty()` early return in
    `crates/smelt-cli/src/bakeoff.rs`) now has **zero reachable test
    coverage** anywhere in this crate — `admitted_family` maps
    `Technique::DeleteInsert` to `None`, so a membership-sensitive cell is
    never a bakeoff candidate, and no other fixture in the workspace admits
    a genuine 2+-technique cell for an `UpstreamMutation` trigger.
  - For a `grain: partition` model (no live runtime dispatch exists for a
    `grain: partition` `DeleteInsert` membership cell —
    `resolve_live_membership_recompute_cell`'s own doc comment), a
    frontmatter `cells[].technique`/`cells[].prefer` pin AND a request-scope
    `ExecuteRequest::technique_overrides` entry are now BOTH silently never
    consulted at all for that cell — not merely "not steering," but not
    even validated: an inadmissible pin (`technique: fold`) that used to
    refuse loudly (`ChoiceRefusal`/`MaintenanceUnboundedFootprint`) now
    succeeds silently, taking the plain default region-recompute path
    regardless of what was pinned/overridden. Verified empirically across
    `bakeoff_seam.rs::request_override_has_no_effect_on_membership_sensitive_cell`,
    `bakeoff_seam.rs::request_override_forces_each_admissible_technique`,
    and `maintenance_pins.rs::inadmissible_pin_has_no_effect_on_membership_sensitive_cell`.

  Both gaps are genuinely new (they did not exist before Phase 1's
  derivation swap — `ColumnScopedMerge` and its pin/override plumbing were
  live and tested then) and are tracked in `docs/TODO.md` rather than
  silently accepted; neither `crates/smelt-cli/src/bakeoff.rs`'s own logic
  nor `crates/smelt-runtime/src/maintenance_driver.rs`'s admission gates
  were modified to paper over them (per this phase's "do not weaken
  bakeoff's own machinery" instruction) — the affected tests were rewritten
  to assert the honest, verified-not-guessed current behavior instead.

  Two `explain_model.rs` tests (`explain_prints_observed_delta_recording_
  status_for_a_conditional_cell`, `explain_prints_no_recording_for_a_whole_
  row_identity_conditional_cell`) were rewritten to build a synthetic
  `MaintenancePlan` directly (mirroring the pre-existing
  `write_variant_explain_surface`/`write_pin_explain_surface` modules'
  pattern in the same file) rather than deriving one from real SQL — the
  EXPLAIN PRINTING logic under test
  (`crates/smelt-cli/src/explain.rs` lines ~353-364) is independent of
  whether real derivation can currently reach `ColumnScopedMerge`, and
  fabricating a contrived SQL shape to keep it artificially reachable would
  have misrepresented what the derivation actually admits today.
  `explain_show_sql.rs::show_sql_statements_unaffected_by_observed_delta_
  report_rows` could not be rewritten the same way (it drives the real
  `smelt` binary as a subprocess against a real project directory, not
  something a synthetic in-process plan can stand in for) — it was narrowed
  to prove the still-true, more general half of its original claim (report-
  section additions/omissions never corrupt the statement section) over the
  model's now-live `DeleteInsert` cell instead.

  **Reviewer pass caught one missed fixture:**
  `crates/smelt-runtime/tests/statement_parity.rs::
  delete_insert_suppressed_keyed_membership_statements_come_from_the_emitter`
  still asserted `group.statements.len() == 5` and diffed against a direct
  `emit_staged_candidate_conditional` call — a leftover from before the
  dispatch was repointed at the new `_recompute` variant's six statements.
  Fixed to assert 6 and diff against `emit_staged_candidate_conditional_
  recompute`; the fixture's own SQL shape (a genuinely mutated dimension,
  no departure) meant the extra departed-key `DELETE` is a no-op there, so
  the test's own result-equivalence assertion was passing for the wrong
  reason until this was caught.

  **Reviewer REQUEST-CHANGES pass (2026-08-08): sibling-cell pin ambiguity,
  found live, fixed.** Gating check 2 of the review caught that the
  "no live dispatch" attribution the previous pass gave for the silent
  `[user_name]`-scoped pins in `maintenance_pins.rs`/`bakeoff_seam.rs` was
  WRONG, proven empirically: renaming the SAME pin's `columns` from
  `[user_name]` to `[event_id]` made it refuse loudly
  (`MaintenanceUnboundedFootprint`). Root cause: `daily_events_enriched`'s
  `UpstreamMutation(users)` trigger derives TWO sibling cells (`{user_name}`
  and `{event_id, event_type, user_id}` — membership sensitivity is
  row-scoped, so a shared join admits a cell per column group, not one cell
  per trigger, `docs/specs/incremental_models.md` §"The plan matrix"), and
  `MaintenancePlan::cell_for`'s first-match lookup meant every pin-
  resolution call site only ever evaluated an override against whichever
  sibling happened to be derived first — a pin scoped to the OTHER
  sibling's own columns was silently never matched, regardless of whether
  it named an admissible or inadmissible technique. This is the exact
  ambiguity Phase 2's own Deferred entry flagged ("`MaintenancePlan::
  cell_for`'s first-match ambiguity (discovered, not introduced)") as a
  finding for a follow-up, now live and fixed under this phase's review
  rather than deferred again.

  Fixed:
  - `crates/smelt-logical/src/maintenance/mod.rs`: `MaintenancePlan` gains
    `cells_for(&self, trigger) -> impl Iterator<Item = &PlanCell>` (every
    sibling cell sharing a trigger); `cell_for`'s own doc comment now warns
    against using it alone to resolve a per-cell override.
  - `crates/smelt-logical/src/maintenance/choice.rs`: `resolve_cell_choice`
    no longer takes `&MaintenancePlan` and does its own internal
    `cell_for(trigger)` lookup — it takes the already-resolved
    `Option<&PlanCell>` directly, so the caller (not this function) is
    responsible for picking the RIGHT sibling. New
    `unaddressed_technique_pin` helper: a HARD `cells[].technique` pin
    naming `on: <trigger>` whose `columns` intersect NONE of a trigger's
    sibling groups is a dangling/misconfigured pin — fail-loud discipline
    (root `CLAUDE.md`) says this must refuse, never silently vanish; a soft
    `prefer` in the same situation is not flagged (it never refuses even
    when it names a resolvable technique the cell lacks — the existing
    contract already covers "no admissible target to steer toward").
  - `crates/smelt-runtime/src/maintenance_driver.rs`: both
    `resolve_live_column_scoped_cell` and `resolve_live_membership_
    recompute_cell`'s per-source loops now collect ALL of a trigger's
    sibling cells (`cells_for`), check for a dangling hard pin across all
    of them up front (loud `bail!` if found), then try each sibling in turn
    — matching `effective_override`/`resolve_cell_choice` against that
    sibling's own group columns, never a different sibling's.
  - New regression test:
    `crates/smelt-logical/src/maintenance/choice.rs::tests::
    pin_scoped_to_a_sibling_cell_is_consulted_not_only_the_first` — a
    2-sibling-cell plan, proving a pin scoped to the SECOND cell's own
    columns is consulted (loud refusal for `Fold`, honored for
    `Recompute`), a pin naming columns absent from EITHER sibling is
    flagged dangling, and the SAME dangling pin as a soft `prefer` is not
    flagged (never refuses).
  - Restored to loud-refusal expectations: `maintenance_pins.rs::
    inadmissible_pin_fails_loud` (was `inadmissible_pin_has_no_effect_on_
    membership_sensitive_cell`), `bakeoff_seam.rs::
    request_override_subject_to_admission` (was `request_override_has_no_
    effect_on_membership_sensitive_cell`), and `bakeoff_seam.rs::
    request_override_forces_each_admissible_technique` — the last one's
    honest post-Phase-1 claim is narrower than its pre-Phase-1 original:
    `recompute` is still always resolvable (unchanged), but `rederive_
    columns` (→ `ColumnScopedMerge`) is now genuinely, permanently
    inadmissible for THIS shape regardless of the sibling-matching fix
    (`ColumnScopedMerge`'s own reachability gap, item #1 in `docs/TODO.md`
    — a real, separate, still-open finding, not the bug this pass fixed),
    so the test now proves `recompute` succeeds and `rederive_columns`
    refuses loud, rather than "both succeed silently."
  - `docs/TODO.md`'s entry corrected: item #2's "no live dispatch" framing
    was true of the write PATH (still is — `grain: partition` genuinely has
    no live `DeleteInsert`-membership dispatch) but was never the reason a
    PIN was silent; the doc now says both things accurately, and item #2 is
    marked resolved (loud refusal restored) rather than open.
  - **NULL-keyed row caveat (advisory, not fixed):** the departed-key
    `DELETE` in `emit_staged_candidate_conditional_recompute` joins on
    plain `=`, so a `NULL`-keyed row is delete+reinserted every run
    (`NULL = NULL` is never true in SQL) — end-state equivalence still
    holds, but change-suppression silently doesn't for that one row. Noted
    on the emitter and tracked in `docs/TODO.md`; not fixed under this pass
    (out of the reviewer's gating scope, and a `COALESCE`-based NULL-safe
    key join risked destabilizing the statement-shape parity legs this late
    in the phase without dedicated red-green coverage of its own).

## Verification

- `cargo test -p smelt-cli --test maintenance_conformance` green, including the dim-mutation pool
- `cargo test -p smelt-runtime --test statement_parity` and `--test technique_lowering` green
- `cargo test -p smelt-logical` green (grouping, tracer, expr_util, walk_coverage)
- `rg -n "collect_column_refs_ungated" crates/` → no hits
- `bash .claude/scripts/verify-phase.sh`
