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
| 1     | pending  |        |      |
| 2     | pending  |        |      |
| 3     | pending  |        |      |
| 4     | pending  |        |      |

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

## Verification

- `cargo test -p smelt-cli --test maintenance_conformance` green, including the dim-mutation pool
- `cargo test -p smelt-runtime --test statement_parity` and `--test technique_lowering` green
- `cargo test -p smelt-logical` green (grouping, tracer, expr_util, walk_coverage)
- `rg -n "collect_column_refs_ungated" crates/` → no hits
- `bash .claude/scripts/verify-phase.sh`
