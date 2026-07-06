# Plan: Maintenance-plan spec alignment (close the §4 spec-diff gaps)

**Date**: 2026-07-07
**Spec**: [`docs/specs/maintenance_plan.md`](../specs/maintenance_plan.md) (plus the specs each phase names)
**Spec diff**: commits `3f65a671` (maintenance_plan), `aa326a3f` (models), `fb9a5977` (sources) — this plan closes the *remaining* rows of the spec-diff map in `docs/research/20260705-refresh-as-maintenance-plan/09-spec-readiness.md` §4 that those commits did not cover
**Tracking PR / branch**: `worktree-incremental`
**Docs**: docs-only <!-- every phase edits docs/specs/ (and CLAUDE.md); no code changes. docs-site rewrite deliberately lives in the implementation plan (after the surface cut lands in code), not here -->

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/maintenance_plan.md` and `docs/specs/models.md` — they are the correctness oracle for every demotion/alignment below. Do not re-open settled spec decisions (the ratification record is `docs/research/20260705-refresh-as-maintenance-plan/09-spec-readiness.md` §1).
2. Confirm you are on branch `worktree-incremental`. If not, ask the user before continuing.
3. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**This is a docs-only plan.** No `crates/` file may be touched. The "TDD tests" for each phase are the mechanical verification greps + `/smelt:validate` listed per phase — run them red (they fail before the edit lands) and green (after).

**When to pause and ask the user:**

- A demotion would delete semantics no other spec now owns (record it in the target spec first, don't drop it).
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).

**Conventions every phase:**

- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push.
- Don't widen scope: a phase may not reach into a later phase's scope.
- **Timeless-oracle rule (CLAUDE.md).** All spec edits describe the feature as if it has always existed; implementation status goes under §Known Divergences with a plan link.

---

## Context

Three of the spec-diff map's rows landed (`maintenance_plan.md` created; `models.md` refresh-axis rewrite; `sources.md` world-facts). Four independent review passes (2026-07-07) confirmed those are faithful to the ratified research and found the residue: the three mode specs are still strategy specs teaching removed `refresh:` values as live surface; `model_transforms.md` lacks the new technique-primitive contracts; the maintenance-plan purity invariant is in neither `architecture.md` nor `CLAUDE.md`; and `diagnostics.md` carries zero `Maintenance*` rows. This plan is that residue — 09-spec-readiness §4's "shape-profile demotions second" plus the three one-file alignments.

## Scope

### In scope (spec coverage)

- `batched_models.md`, `keyed_models.md`, `versioned_models.md`: demotion from strategy specs to **shape profiles** per `models.md` §"Refresh axis" + §Design ("Strategy content is derived; shape and grain stay declared") — each becomes: the grain it profiles, its default plan, its local machinery, admission re-derived as instances of `maintenance_plan.md` §"Per-cell admission" failure cases.
- `model_transforms.md`: the technique-primitive contracts named by `maintenance_plan.md` §Interactions — generic column-scoped merge, ledger fold + recompute-reset, and the definition-change field-backfill pair (in-place `UPDATE` vs keyed column-scoped `MERGE`, skeleton-position refusal).
- `architecture.md` + `CLAUDE.md`: the plan-purity invariant already asserted by `maintenance_plan.md` §Constraints ("pure data, derived by pure functions, in one place; consumers never re-derive").
- `diagnostics.md`: the `Maintenance*` family rows (the six codes specced in `maintenance_plan.md` §Diagnostics, plus the family members from `06-proof-obligations.md` §7 that other specs own).

### Explicitly deferred

- The docs-site refresh-surface rewrite — lives in `docs/plans/20260707-maintenance-plan-impl.md` phase MP2, *after* the parser accepts the trichotomy (user docs must not describe surface the CLI rejects).
- `columns.<c>.contract` grammar ownership vs future column `tests:` — deliberately deferred by ratified decision 8; do not spec it here.
- Any code change (including the `RefreshStrategy` cut) — implementation plan.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| SA1   | pending  |        |      |
| SA2   | pending  |        |      |
| SA3   | pending  |        |      |
| SA4   | pending  |        |      |
| SA5   | pending  |        |      |

---

### Phase SA1: Demote `batched_models.md` to the partition-grain shape profile

**Goal.** `batched_models.md` becomes the shape profile for `refresh: incremental` + `grain: partition`: what the shape is, its default plan (recompute-a-region per partition), and the batched-local machinery (batch-safety classes, backfill chunking, self-referential ordered execution, monotone-integer partition columns) — with admission presented as instances of `maintenance_plan.md` §"Per-cell admission", not a freestanding matrix.

**Pre-conditions.** `models.md` §"Refresh axis" and `maintenance_plan.md` are committed (they are).

**Verification greps (red before, green after).**
- `rg -n 'refresh: batched' docs/specs/batched_models.md` → only inside §Known Divergences / historical notes, never in §Surface examples.
- `rg -n 'nondeterministic_columns' docs/specs/batched_models.md` → survives only as a Known-Divergence pointer to `columns.<c>.contract` (`models.md` owns the contract surface).
- `/smelt:validate batched_models` reports no timeless-oracle violations.

**Implementation shape.** Rewrite §Surface to the profile form (frontmatter example uses `refresh: incremental` + `grain: partition` + `timeseries:`); keep §Semantics that is genuinely partition-local (batch-safety classification, chunking, ordered self-reference); replace the admission matrix with a table mapping each old admission row to the `maintenance_plan.md` obligation it instantiates; move "what the parser accepts today" into §Known Divergences (one snapshot, dated, citing the impl plan). Reconcile the stale `RefreshStrategy` enum snapshot.

**Critical files (allowed to touch in this phase).**
- `docs/specs/batched_models.md` — the rewrite
- `docs/specs/models.md` — only if a cross-reference needs updating (no semantic change)

**Docs touched.** *Timeless — no phase vocabulary.*
- `docs/specs/batched_models.md` — full demotion rewrite

**Review checklist** (material findings only):
- [ ] No semantics dropped that no other spec owns (batch-safety, chunking, ordered self-reference all survive)
- [ ] Admission rows each cite the `maintenance_plan.md` obligation they instantiate
- [ ] `refresh: batched` appears only as removed-surface history
- [ ] Spec edits are timeless

**Commit.** `spec(batched): demote to the partition-grain shape profile; admission as per-cell obligations`

---

### Phase SA2: Demote `keyed_models.md` to the key-grain shape profile

**Goal.** Same demotion for `keyed_models.md`: the shape profile for `refresh: incremental` + `grain: key` (and `key_per_partition` where §"Key temporal locality" applies). Column families, derived postures, the merge ledger, once-write and the pattern functions stay — they are the key-grain default plan and local machinery — but the mode-local admission matrix is re-derived as `maintenance_plan.md` obligation instances, and `refresh: keyed` moves to removed-surface history.

**Pre-conditions.** SA1 (establishes the profile form to mirror).

**Verification greps (red before, green after).**
- `rg -n 'refresh: keyed' docs/specs/keyed_models.md` → only §Known Divergences / history.
- `/smelt:validate keyed_models` reports no timeless-oracle violations.

**Implementation shape.** As SA1. Note the keyed spec is the youngest (2026-07-05 collapse) — most §Semantics content survives verbatim; the work is the surface reframe + admission re-derivation + reconciling its `RefreshStrategy` snapshot with `models.md`'s.

**Critical files (allowed to touch in this phase).**
- `docs/specs/keyed_models.md` — the rewrite

**Docs touched.** *Timeless.*
- `docs/specs/keyed_models.md` — demotion rewrite

**Review checklist** (material findings only):
- [ ] Column-family catalogue, postures, ledger, once-write semantics all survive
- [ ] Admission re-derived, citing per-cell obligations
- [ ] Spec edits are timeless

**Commit.** `spec(keyed): demote to the key-grain shape profile; admission as per-cell obligations`

---

### Phase SA3: Demote `versioned_models.md` to the SCD-2 shape profile

**Goal.** `versioned_models.md` becomes the SCD-2 shape profile (key-grain output with smelt-managed validity columns; default plan: keyed close-old/open-new `merge_into` — the out-of-window keyed write). `refresh: versioned` moves to removed-surface history; the stale sibling-rule sketch pointer (`scd2/latest_value/accumulating_snapshot`) is cleaned up.

**Pre-conditions.** SA2.

**Verification greps (red before, green after).**
- `rg -n 'refresh: versioned' docs/specs/versioned_models.md` → only §Known Divergences / history.
- `rg -n 'latest_value|accumulating_snapshot' docs/specs/versioned_models.md` → only as historical/decision-record citations.
- `/smelt:validate versioned_models` clean.

**Critical files (allowed to touch in this phase).**
- `docs/specs/versioned_models.md` — the rewrite

**Docs touched.** *Timeless.*
- `docs/specs/versioned_models.md` — demotion rewrite

**Review checklist** (material findings only):
- [ ] Validity-column semantics and tracked-attribute selection survive
- [ ] Spec edits are timeless

**Commit.** `spec(versioned): demote to the SCD-2 shape profile; admission as per-cell obligations`

---

### Phase SA4: `model_transforms.md` — the maintenance technique-primitive contracts

**Goal.** Catalogue the transform contracts `maintenance_plan.md` §Interactions promises: (a) the **generic column-scoped merge/update** primitive (of which the dimension-driven horizon MERGE is the existing instance); (b) the **ledger pair** — fold (refuse a delta already in the entry's processed set) and **recompute-reset** (a region recompute resets every intersecting entry to exactly what it read); (c) the **definition-change field-backfill** pair — in-place `UPDATE` under the additive-only model-diff proof vs keyed column-scoped `MERGE` when re-deriving from upstream, with the skeleton-position refusal (`MaintenanceSkeletonColumnAdded`) cross-referenced, per `maintenance_plan.md` §"The definition-change trigger". Also fix the stale summary-table row that still says the outer clamp "filters the outermost SELECT" (the prose already says subquery wrap).

**Pre-conditions.** None beyond committed specs.

**Verification greps (red before, green after).**
- `rg -n 'recompute-reset|recompute reset' docs/specs/model_transforms.md` → present in the catalogue.
- `rg -n 'skeleton' docs/specs/model_transforms.md` → the field-backfill contract names the refusal.
- Summary table row for the output clamp matches §Semantics (subquery wrap).

**Critical files (allowed to touch in this phase).**
- `docs/specs/model_transforms.md`

**Docs touched.** *Timeless.*
- `docs/specs/model_transforms.md` — catalogue + contract additions; unbuilt mechanisms marked in the existing maturity vocabulary (`unbuilt`), not phase labels

**Review checklist** (material findings only):
- [ ] Contracts state obligations + failure modes, not implementations
- [ ] No duplication of `maintenance_plan.md` semantics — contracts cite it
- [ ] Spec edits are timeless

**Commit.** `spec(transforms): column-scoped merge, ledger fold/reset, and field-backfill as transform contracts`

---

### Phase SA5: Architecture invariant + `Maintenance*` diagnostics catalogue

**Goal.** (a) Land the plan-purity invariant in `architecture.md` §Constraints & Invariants and mirror it into `CLAUDE.md` §Architectural invariants: *the maintenance plan is pure data in `smelt-logical`, derived by pure functions; consumers (smelt-db diagnostics, smelt-planner application, smelt-runtime lowering, the graph layer) never re-derive it* — with the structural assertion the invariant will be checked by. (b) Add the `Maintenance*` family to `diagnostics.md`'s catalogue: the six codes from `maintenance_plan.md` §Diagnostics, grouped under an "owned by `maintenance_plan.md`" section, each row citing the owning spec; a §Known Divergences note records that the family is specified-and-unimplemented (no enum variants yet — the catalogue gate is enum→catalogue, so rows may precede variants), linking `docs/plans/20260707-maintenance-plan-impl.md`. (c) Add the missing reverse cross-reference from `model_properties.md` §References to `maintenance_plan.md` (its newest consumer).

**Pre-conditions.** None.

**Verification greps (red before, green after).**
- `rg -n 'maintenance plan' docs/specs/architecture.md CLAUDE.md` → the invariant present in both.
- `rg -c 'Maintenance' docs/specs/diagnostics.md` → ≥ 6 catalogue rows.
- `rg -n 'maintenance_plan' docs/specs/model_properties.md` → cross-reference present.
- `cargo test -p smelt-db --test diagnostics_catalogue` still green (no enum change).

**Critical files (allowed to touch in this phase).**
- `docs/specs/architecture.md`, `CLAUDE.md`, `docs/specs/diagnostics.md`, `docs/specs/model_properties.md`

**Docs touched.** *Timeless.*
- as above

**Review checklist** (material findings only):
- [ ] Invariant wording matches `maintenance_plan.md` §Constraints (one source of truth, cross-referenced)
- [ ] Catalogue rows cite the owning spec, don't restate semantics
- [ ] Spec edits are timeless

**Commit.** `spec(architecture+diagnostics): maintenance-plan purity invariant; Maintenance* catalogue family`

---

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this plan.)

## Verification

- `rg -n 'refresh: (batched|keyed|cumulative|versioned)' docs/specs/*.md` → matches only in Known-Divergence/history contexts and `models.md`'s removed-values table.
- `/smelt:validate models`, `/smelt:validate maintenance_plan`, `/smelt:validate batched_models`, `/smelt:validate keyed_models`, `/smelt:validate versioned_models` — zero drift.
- `cargo test -p smelt-db --test diagnostics_catalogue` green.
