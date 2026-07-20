# Plan: Production W10 — keyed mutable-source admission + suppressed-MERGE closure

**Date**: 2026-07-20
**Spec**: [`docs/specs/incremental_models.md`](../specs/incremental_models.md) §"Admission matrix", §"The key grain (`grain: key`)" (input-consumption obligations), §"Per-cell write addressing", §Known Divergences
**Spec diff**: Phase 1 of this plan (narrow the key-grain `NewData` append-only obligation); Phases 4–5 close the same divergences W8's 5a/5b would have closed
**Tracking PR / branch**: `worktree-production`
**Docs**: code+docs
**Master**: [`docs/plans/20260719-production-readiness.md`](20260719-production-readiness.md) (sub-plan W10)
**Supersedes**: `docs/plans/20260719-prod-w8-composed-axes-followups.md` Phases 5a + 5b — this plan absorbs the runtime dispatch and the generative conformance leg, because both were unreachable until the admission gap below is closed. W8's independent Phase 6 is already `done`; its 5a/5b rows are marked `superseded` and point here.

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session. Your job is to drive it to completion using `/smelt:implement`.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/incremental_models.md` §"Admission matrix", §"The key grain (`grain: key`)" (the input-consumption obligation list), and §"Per-cell write addressing" — they are the correctness oracle. The **equivalence invariant** (`incremental_state(S) == full_refresh(inputs ∈ S)`) is the thing every phase must not break. Do not re-open settled spec decisions.
2. Confirm you are on branch `worktree-production`. If not, ask the user before continuing.
3. **Pre-flight.** `rg -n 'carries_retractions' crates/smelt-logical/src/maintenance/derive.rs` must hit the append-only refusal this plan narrows (≈ derive.rs:699-747). If it does not, the code has moved — re-locate it with one targeted read before proceeding; do not guess.
4. Find the next phase whose status is `pending` in the Progress tracking table. That is your starting point. If every phase is `done`, run the post-implementation verification under "Verification" and stop.

**For each phase, run the per-phase loop encoded in `/smelt:implement`:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**When to pause and ask the user:**

- The reviewer surfaces the same **admission-safety** finding across two implementer passes (this plan touches admission logic — a false relaxation is a silent correctness hole; escalate rather than paper over it).
- TDD tests cannot be made green without violating the equivalence invariant or the property-composition-walk rule.
- A spec assumption turns out to be wrong (run `/smelt:spec` to update first).
- `cargo test` or `cargo clippy` surfaces a pre-existing failure unrelated to the plan.

**Conventions every phase:**
- Red-green TDD; real-fixture tests where the phase has user-visible behaviour.
- Verification gate is `bash .claude/scripts/verify-phase.sh` (one call; failures-only output). Standing gates this plan must keep green: `execute_parity`, `statement_parity`, `maintenance_conformance`, `walk_coverage`, plus the hardening/census/registry ratchets.
- Atomic per-phase commits with the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking PR.
- Timeless-oracle rule: spec and docs-site edits carry no phase vocabulary.

---

## Context

W8 Phase 5a/5b tried to make the **change-suppressed column-scoped `MERGE`** reachable on a generatable keyed recipe: a composed clock-and-identity `grain: key` model that enriches from an `explicitly_mutable` dimension declared `allow_full_scan`. The derivation already produces the right `Technique::ColumnScopedMerge` cell carrying `RowIdentity::Key` for that shape, and 5a's runtime dispatch (consult `resolve_live_column_scoped_cell` on the keyed branch of `execute.rs`) compiled clean with all gates green — **but it has no reachable red test**, because the model never gets past `execute_project`'s pre-execution diagnostic gate. It dies with `MaintenanceNoAdmissibleTechnique`.

**Root cause (verified 2026-07-20).** `derive_new_data`'s `Grain::Key` branch (`crates/smelt-logical/src/maintenance/derive.rs:699-747`) refuses the whole plan for a `Trigger::NewData { source }` whenever `facts.mutation != MutationProfile::AppendOnly` — keyed **only** on that one field. It does not consult `allow_full_scan`, and it does not check whether the source actually feeds the cumulative fold's combiner. The trigger list (`crates/smelt-db/src/queries/maintenance.rs:364-406`) unconditionally pushes one `NewData` trigger per referenced source, so *any* non-append-only source a keyed model touches — including a mutable dimension it only enrich-joins — trips the refusal. That refusal maps to an unconditional `Error` diagnostic (`smelt-db/src/lib.rs`), blocking `smelt build`/`execute_project`.

**Why this is a real admission gap, not just a test nuisance.** The append-only obligation on the key grain guards the *fold*: a cumulative combiner over a mutable input could silently miss a retraction (§"The key grain" input-consumption obligations 1–2). But a mutable source consumed **only** via a covered `UpstreamMutation`/`ColumnScopedMerge` enrichment cell is *not folded* — its post-creation mutations are maintained by the merge cell, which is exactly the trigger that dispatch was meant to reach. Refusing on such a source is the admission logic double-counting one source under two triggers and rejecting on the one it already covers elsewhere.

**Why the obvious narrowing is unsafe.** Keying the relaxation purely off "this source is covered by an `UpstreamMutation` cell" is **wrong**, because a single mutable source can *simultaneously* feed a fold combiner (`SUM(dim.amount)`) and be enrich-joined. The derived plan cannot currently tell those apart: `FoldSpec.add_columns` (`derive.rs:125-128`) is `Vec<(alias, combiner)>` with **no source attribution** — `derive_fold_spec` (`crates/smelt-db/src/queries/maintenance.rs:133-151`) scans the SQL for `(alias, combiner)` and discards which source each aggregate reads. Relaxing without proving the source is *not* a fold input would admit an un-retractable folded contribution — a direct equivalence-invariant violation.

**This plan's fix.** Add the missing safety fact as a **conservative fold-contribution predicate** — "does this source appear as an argument to any fold aggregate?" — computed as a leaf classifier over the model's own fold body (property-composition-walk rule: a leaf classifier the walk invokes over one already-bounded node's own text is admissible; it must be doc-commented as such). Narrow `derive_new_data` to waive the append-only obligation for a source **iff** it is covered by an `UpstreamMutation` cell **and** does not contribute to the fold; a source that is both stays refused (fail-loud, conservative — that folded contribution genuinely *is* un-retractable). Then reintroduce W8 5a's runtime dispatch against a now-reachable red test, and land W8 5b's generative conformance leg on top.

## Scope

### In scope
- `incremental_models.md` §"Admission matrix" / §"The key grain": the `NewData` append-only obligation is stated to bind **fold-contributing** sources, not every referenced source; a mutable source consumed only via a covered enrichment cell is admitted; a mutable source that both feeds the fold and is enrich-joined stays refused.
- `crates/smelt-logical/src/maintenance/` — a fold-contribution classifier and the narrowed `derive_new_data` refusal, threaded with the precomputed covered-source set.
- `crates/smelt-runtime/src/execute.rs` — the keyed-branch change-suppressed column-scoped `MERGE` dispatch (W8 5a's reverted change, now testable).
- `crates/smelt-cli/tests/maintenance_conformance/**` — the generative suppressed-MERGE equivalence leg (W8 5b).
- `incremental_models.md` §Known Divergences — the entries scoping live column-scoped-`MERGE` dispatch to the partition/incremental path, and the C4/E4 "hand-built fixtures only" conformance caveat, both narrow.

### Explicitly out of scope / deferred
- **Full fold source-attribution** (extending `FoldSpec`/`derive_fold_spec` to map each `add_columns` entry to its backing source via column-origin resolution). The conservative predicate here refuses the both-fold-and-enrich overlap outright; the more-permissive version that admits it when the fold contribution is provably additive-with-retraction is a future optimisation, tracked here, demand-gated on a real workspace needing it.
- **`Grain::Partition` enrichment `RowIdentity::Key`** (W8's candidate (a): threading `SourceFacts`/`JoinContext` into P2 for partition-grain joins). Untouched — larger blast radius, no current consumer.
- Anything W9 (Spark twin) owns.

---

## Phases

### Phase 1: Spec diff — the append-only obligation binds fold-contributing sources

**Goal.** `incremental_models.md` states normatively that the key grain's `NewData` append-only obligation binds **fold-contributing** sources — a source whose deltas the cumulative combiner folds — not every referenced source. A mutable source consumed **only** via a covered `UpstreamMutation`/`ColumnScopedMerge` enrichment cell does not trip the obligation (its mutations are maintained by that cell). A source that **both** feeds the fold and is enrich-joined stays refused (`MaintenanceNoAdmissibleTechnique`) — the folded contribution is un-retractable, and the conservative admission refuses it fail-loud. The §Known Divergences entry that scopes live column-scoped-`MERGE` dispatch to the partition/incremental path is narrowed to say the keyed run path dispatches it too.

**Pre-conditions.** Pre-flight passed. Docs-only phase.

**TDD tests to write first.** None (docs-only). Phases 2–4 write tests against this text.

**Implementation shape.** Locate the input-consumption obligation list under §"The key grain (`grain: key`)" (obligations 1–2, "Replayable input / faithful fold") and the §"Admission matrix" entry that states the append-only requirement — one targeted read each. Rewrite them to bind the obligation to **fold-contributing** sources, and add the carve-out sentence: a mutable source consumed only through a covered enrichment (`UpstreamMutation`) cell is admitted; a source that is both a fold input and mutable is refused. In §Known Divergences, narrow the "live column-scoped `MERGE` dispatch is reachable only on the incremental/partition path" entry to include the keyed run path. All timeless — describe behaviour, no phase vocabulary.

**Critical files (allowed to touch in this phase).**
- `docs/specs/incremental_models.md` — the obligation list, the admission-matrix entry, the divergence entry.

**Docs touched.** *(timeless)*
- Spec only; docs-site rides with Phase 4 when the runtime behaviour lands.

**Review checklist** (material findings only):
- [ ] The obligation is stated over fold-contributing sources, not "every referenced source"
- [ ] The both-fold-and-enrich case is explicitly still refused (no silent admission)
- [ ] The divergence entry describes the keyed-path dispatch behaviourally; no phase vocabulary

**Commit.** `docs(spec): key-grain append-only obligation binds fold-contributing sources, not every referenced source`

### Phase 2: Fold-contribution classifier

**Goal.** A pure predicate `source_contributes_to_fold(fold: &FoldSpec-or-body, source: &str) -> bool` (final shape decided in implementation) answers "does this source appear as an argument to any aggregate the fold folds?" for the model's own fold body. It is the safety fact the narrowing in Phase 3 needs and the derived plan does not currently carry. Implemented as a **leaf classifier over one already-bounded node's own text** (the model's fold/aggregation body) — admissible under the property-composition-walk rule and doc-commented as such (it feeds admission, so the classification note is mandatory, not optional).

**Pre-conditions.** Phase 1 merged.

**TDD tests to write first.**
- `crates/smelt-logical/tests/` (or the maintenance module's unit tests) — table-driven:
  - `SUM(dim.amount)` with `dim` the source ⇒ `true` (dim feeds the fold).
  - a fold that reads only `fact.*` while `dim` appears **only** in a `JOIN ... ON`/`SELECT` enrichment column, never inside an aggregate ⇒ `false` for `dim`.
  - a source aliased in `FROM` and referenced via that alias inside an aggregate ⇒ `true` (alias resolution counted; if alias resolution is out of scope for the leaf classifier, the conservative answer is `true` — assert that conservatism explicitly so the test pins the fail-safe direction).
  - qualified vs bare column references handled the same way.
- The test comment states the invariant: **false negatives are forbidden** (a source that does feed the fold must never classify `false`) because a false negative is the admission hole; false positives only cost permissiveness (a genuinely-enrich-only source refused) and are acceptable.

**Implementation shape.** Add the classifier next to the fold machinery in `crates/smelt-logical/src/maintenance/` (or `derive.rs`). It scans the fold's aggregate expressions for column references naming (or aliased to) the source. Bias every ambiguity toward `true` (contributes) so the predicate can only be conservative, never permissive. Doc comment must classify it per the property-composition-walk rule: *"leaf classifier — runs over the model's own already-bounded fold body; feeds admission; never composes across nodes."* No change to `derive_new_data` yet (that is Phase 3, so this phase's predicate is dead-but-tested until wired).

**Critical files (allowed to touch in this phase).**
- `crates/smelt-logical/src/maintenance/derive.rs` (or a sibling module) — the classifier + its unit tests.

**Docs touched.** *(timeless)* — none (internal predicate).

**Review checklist** (material findings only):
- [ ] The classifier is conservative: every ambiguous case resolves to `true` (contributes), asserted by a test
- [ ] Doc comment classifies it as a leaf classifier per the property-composition-walk rule
- [ ] `walk_coverage` gate green (the classifier is a leaf, not a competing scan)

**Commit.** `feat(maintenance): fold-contribution leaf classifier — does a source feed the cumulative fold`

### Phase 3: Narrow the `derive_new_data` append-only refusal

**Goal.** `derive_new_data` waives the append-only refusal for a `NewData { source }` trigger **iff** the source is (i) covered by an `UpstreamMutation` cell for this model **and** (ii) not fold-contributing (Phase 2). Every other non-append-only source still refuses exactly as today. The keyed + mutable-dim + `allow_full_scan` model now passes admission; the both-fold-and-enrich overlap model still refuses.

**Pre-conditions.** Phase 2 merged (the classifier exists and is tested).

**TDD tests to write first.**
- `crates/smelt-logical/tests/` (derivation unit, against `derive_model_maintenance_plan` / `derive_maintenance_plan_impl`):
  - **Red (admit):** a `grain: key` model, `unique_key` set, enriching a `mutation_profile: mutable_snapshot` dimension declared `allow_full_scan`, dim **not** inside any fold aggregate ⇒ plan carries **no** `NoAdmissibleTechnique` refusal for that source, and still carries the `ColumnScopedMerge` cell for it. Fails today (blanket refusal fires).
  - **Guard (still refuse — the safety case):** same model but the fold now aggregates the mutable dim (`SUM(dim.amount)`) ⇒ plan **still** carries the `NoAdmissibleTechnique` refusal (fold contribution is un-retractable). This test must stay green through the narrowing — it pins the safety carve-out.
  - **Unchanged:** a mutable source with **no** `UpstreamMutation` cell (e.g. not `explicitly_mutable`) ⇒ still refused (coverage is a necessary condition).
- `crates/smelt-cli/tests/maintenance_conformance` — the standing keyed conformance legs stay equal to the full-refresh oracle (no regression from the relaxed admission).

**Implementation shape.** In `crates/smelt-db/src/queries/maintenance.rs`, precompute the covered-source set from the same predicate that builds the `UpstreamMutation` triggers (≈ L397-404: `partition_col.is_none() && mutation == MutableSnapshot && explicitly_mutable.contains(name)`) and pass it into the derivation. In `crates/smelt-logical/src/maintenance/derive.rs`, extend `derive_new_data`'s signature to receive the covered-source set (and the fold, already on `ModelInputs`), and gate the `carries_retractions` refusal (derive.rs:699-747): skip the refusal only when `covered.contains(source) && !source_contributes_to_fold(fold, source)`. Do **not** reorder the trigger loop and do **not** read `plan.cells` (the mutation cell is not derived yet at this point — pass the precomputed set instead). No change to `derive_mutation` or the emit layer.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-db/src/queries/maintenance.rs` — precompute + thread the covered set.
- `crates/smelt-logical/src/maintenance/derive.rs` — the narrowed refusal + signature.

**Docs touched.** *(timeless)*
- `docs/specs/diagnostics.md` — if the `MaintenanceNoAdmissibleTechnique` entry describes the trigger condition, update it to "a fold-contributing non-append-only source" (behavioural).

**Review checklist** (material findings only):
- [ ] The waiver requires **both** coverage and non-contribution — neither alone admits
- [ ] The both-fold-and-enrich guard test is green (safety carve-out holds)
- [ ] No trigger-loop reorder, no `plan.cells` read in `derive_new_data`, no `derive_mutation`/emit change
- [ ] `statement_parity` + `maintenance_conformance` + `walk_coverage` green

**Commit.** `feat(maintenance): key-grain NewData waives append-only for enrich-only covered mutable sources`

### Phase 4: Dispatch change-suppressed column-scoped MERGE on the keyed run path

**Goal.** Reintroduce W8 5a's runtime dispatch, now with a reachable red test: a composed clock-and-identity **keyed** model that enriches from an `explicitly_mutable` dimension declared `allow_full_scan` maintains its dimension-driven column group via the change-suppressed column-scoped `MERGE` at runtime, reaching `Suppressed` when P2 (declared `unique_key` → `RowIdentity::Key`) and P3 (per-column change comparability) both hold. This is a Known-Divergence closure, not new normative surface (the spec already describes the behaviour as live; Phase 1 narrowed the divergence entry).

**Pre-conditions.** Phase 3 merged (admission now lets the model reach execution). DuckDB backend only — no Spark.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/technique_lowering.rs` (sibling of `column_scoped_merge_e2e`) — the keyed model above: after a dimension mutation that genuinely changes a compared column, the dimension-driven column is column-scoped-merged and the **`Suppressed`** arm (`IS DISTINCT FROM`) executes; a no-change redelivery writes nothing. Assert the technique reached is `ColumnScopedMerge` + `Suppressed`, not the cumulative fold, not `Unconditional`. This is now the reachable red test (it compiles *and runs* to `execute_project` because Phase 3 admits the model).

**Implementation shape.** In `crates/smelt-runtime/src/execute.rs`, `plan_is_keyed` branch: before the unconditional return (~L1610), for the keyed model's `explicitly_mutable` sources, call `resolve_live_column_scoped_cell` exactly as the incremental branch does (~L1788) and dispatch the resolved column-scoped `MERGE` (with its `WriteSuppression`) alongside the cumulative fold when a live mutation cell resolves and the target table exists. The cumulative fold still owns the `NewData` (creation/append) trigger; the column-scoped merge owns the `UpstreamMutation` trigger. **No new emit code** — reuse the single-owner `smelt-logical::maintenance::emit` column-scoped-merge emitters (statement-parity gate). Not `derive.rs`, not the emit layer.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/execute.rs`, `crates/smelt-runtime/src/cumulative.rs` (dispatch helper if cleaner), `crates/smelt-runtime/src/maintenance_driver.rs` (only if a small shared helper is needed), the runtime test above.

**Docs touched.** *(timeless)*
- `docs/specs/incremental_models.md` §Known Divergences — if any residual text still scopes the dispatch to the non-keyed path after Phase 1, finish narrowing it.

**Review checklist** (material findings only):
- [ ] The keyed model reaches `ColumnScopedMerge` + `Suppressed` at runtime (the e2e Suppressed assertion)
- [ ] No-change redelivery writes nothing (suppression actually suppresses)
- [ ] The creation/append fold still runs on the keyed path — standing keyed conformance legs stay green
- [ ] `statement_parity` + `technique_lowering` green (single-owner emission preserved); no `derive.rs`/emit change

**Commit.** `feat(runtime): dispatch change-suppressed column-scoped MERGE on the keyed run path`

### Phase 5: Generative conformance leg for change-suppressed column-scoped MERGE

**Goal.** Close the C4 deferred item: at least one generated `maintenance_conformance` recipe resolves `RowIdentity` to a proven/declared grain key so `resolve_write_suppression` genuinely admits `Suppressed`, and the suppressed-vs-full-refresh equivalence is proven **generatively**, not only on hand-built fixtures. Adds no production code (Phase 4 landed the dispatch; Phase 3 the admission).

**Pre-conditions.** Phase 4 done.

**TDD tests to write first.**
- `crates/smelt-cli/tests/maintenance_conformance/gate.rs` — a structural leg asserting the recipe pool contains at least one recipe whose derived plan admits `Technique::ColumnScopedMerge` with `Suppressed` resolved (guards against silent degradation back to `Unconditional`-only — the exact failure mode the source plan recorded). Verify it fails when admission is temporarily broken.
- `crates/smelt-cli/tests/maintenance_conformance` — the equivalence run over the new recipe family: after every step, including an unchanged-input redelivery step (the zero-write case), state equals the full-refresh oracle.

**Implementation shape.** Add a recipe (or extend the pool) whose model is the keyed shape Phase 4 dispatches: `grain: key` (top-level `unique_key:`) enriching from a `mutable_snapshot` dimension declared `allow_full_scan`, with the dim **not** aggregated by the fold (so Phase 3 admits it and Phase 2's classifier returns `false` for it). The redelivery step must be a genuine no-change delta so the suppression arm executes. Runs on DuckDB (and, via W9's backend seam, dual-backend where Spark is live). No production code — if admission still falls short, that is a finding against Phase 3, not new scope here.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-cli/tests/maintenance_conformance/**` — recipe + gate legs only.

**Docs touched.** *(timeless)*
- `docs/specs/incremental_models.md` — narrow the conformance-posture caveat that scoped the C4/E4 evidence to hand-built fixtures: the change-suppressed column-scoped MERGE now has its generative equivalence leg.

**Review checklist** (material findings only):
- [ ] The structural leg fails if no recipe admits `Suppressed` (verified by temporarily breaking admission)
- [ ] The equivalence leg includes the zero-write redelivery step
- [ ] No production code changed in this phase

**Commit.** `test(conformance): generative suppressed-MERGE equivalence leg via keyed declared-key recipe`

---

## Progress tracking

| Phase | Status  | Commit | Date |
|-------|---------|--------|------|
| 1     | done    | `docs(spec): key-grain append-only obligation binds fold-contributing sources, not every referenced source` | 2026-07-20 |
| 2     | done    | `feat(maintenance): fold-contribution leaf classifier — does a source feed the cumulative fold` | 2026-07-20 |
| 3     | pending |        |      |
| 4     | pending |        |      |
| 5     | pending |        |      |

---

## Verification

After all phases:
- `bash .claude/scripts/verify-phase.sh` green (fmt + clippy + full test + example_diagnostics).
- `cargo test -p smelt-runtime --test statement_parity` and `--test execute_parity` green.
- `cargo test -p smelt-cli --test maintenance_conformance` green, including the new suppressed-MERGE family and its zero-write redelivery step.
- `cargo test -p smelt-logical --test walk_coverage` green (the fold-contribution classifier is a leaf, not a competing scan).
- Manual oracle check: the both-fold-and-enrich model still emits `MaintenanceNoAdmissibleTechnique` (the safety carve-out did not regress into silent admission).
- W8 Phases 5a/5b marked `superseded` in `docs/plans/20260719-prod-w8-composed-axes-followups.md` pointing here; W8's master-registry row is `done`.

## Blocked phases

*(none yet)*
