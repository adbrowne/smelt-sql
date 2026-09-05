# Outcome: Programme hygiene — one consistent record of the incremental programme

**Created:** 2026-09-04
**Status:** done
**Source:** `docs/research/20260904-incremental-state-review.md` §"Recommended next sequence" items 5 and 6 (record half); `docs/TODO.md` §"Follow-ups from the 2026-08-12 incremental spec re-architecture"; `docs/specs/state.md` §Known Divergences bullet 4
**Spec anchors:** `docs/specs/state.md`, `docs/specs/run_state.md`, `docs/specs/sources.md`, `docs/specs/schema_evolution.md` (or whichever spec owns schema snapshots), `docs/specs/model_properties.md`

## The outcome

A fresh session that reads the handoffs, the backlog and the specs finds one statement of the
incremental programme, not two. The 2026-08-16 handoff is marked superseded with a pointer to the
2026-09-04 review, and the backlog reflects the sequence that review recommends. Every stale spec
citation `docs/TODO.md` lists is re-pointed or deleted. Each state structure whose owning spec
does not yet say what happens when it is absent gets its one sentence, as `state.md`'s
optionality rule requires. The timeseries enriched-events fixture's doc comment describes the
technique the fixture actually derives. This is a docs-only outcome: no crate changes.

## Success criteria (checkable)

1. `docs/handoffs/2026-08-16-delta-signature-closure-programme.md` carries a dated
   "Superseded" banner at the top linking `docs/research/20260904-incremental-state-review.md`,
   and its queue section is not restated anywhere as current.
2. `.claude/outcome-backlog` lists every `done`/`blocked` outcome after every queued one, in the
   review's recommended order, with a comment line naming the review as the ordering source.
3. Every citation in `docs/TODO.md` §"Stale citations flagged by the sweep" either points at a
   heading that exists (verified by `rg` for the heading text) or is deleted along with the
   sentence that depended on it; the TODO bullet itself is removed.
4. Schema snapshots, source postures and probe baselines each have one sentence in their owning
   spec stating absent-state behaviour, and `state.md` §Known Divergences bullet
   "Structure-level degradation behaviours are unevenly specified" is deleted.
5. `examples/timeseries/models/daily_events_enriched.sql`'s comment matches the technique
   `smelt explain` reports for its `{user_name}` cell (currently `DeleteInsert`), and
   `docs/TODO.md`'s note about that inaccuracy is removed.
6. `/smelt:validate state` and `/smelt:validate model_properties` report no drift introduced
   by this outcome; `bash .claude/scripts/verify-phase.sh` green (doc-sync gates included).

## Out of scope

- Any code change. If a stale citation reveals a behaviour gap, record it under
  `docs/TODO.md` and move on.
- The docs-site delta-signature pass (`docs/outcomes/20260904-delta-signature-front-door`).
- Reopening any decision recorded in `docs/research/20260816-open-questions-triage.md`.
- `.claude/active-plan` (still names the production-readiness programme). It is the *autonomy*
  loop's pointer and is branch-scoped by its own header; editing it from this worktree invites a
  cross-worktree conflict with `worktree-production`, and the handoff's own §Pointers already
  flags it as stale for any reader who arrives there.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Supersede the 2026-08-16 handoff with a banner + pointer; confirm `.claude/outcome-backlog` order against the review; re-point the one dangling citation to a never-scaffolded outcome directory | done |
| 2 | Re-point or delete every stale citation listed in `docs/TODO.md`; remove that TODO bullet | done |
| 3 | Add absent-state sentences for schema snapshots, source postures, probe baselines in their owner specs; delete the `state.md` bullet | done |
| 4 | Correct the `daily_events_enriched.sql` fixture comment and the docs-site transcript that repeats its claim; remove the TODO note | done |
| 5 | Correct `daily_events_status.sql` + `user_status.yml`'s `ColumnScopedMerge` overclaim to the derived `DeleteInsert`; remove that TODO bullet | done |
| 6 | Validate: `/smelt:validate state` + `model_properties` clean, verify-phase green | done |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

- 2026-09-04 (plan 01): `.claude/outcome-backlog` already carries the review-sourced ordering
  comment and lists every `done`/`blocked` outcome after every queued one (added when the
  2026-09-04 outcomes were scaffolded), so criterion 2 becomes a *verification* task in phase 1
  rather than a rewrite.
- 2026-09-04 (plan 01): phase 1 widened to re-point `docs/specs/run_state.md` line 175, the only
  citation anywhere to a never-scaffolded outcome directory
  (`docs/outcomes/20260816-scheduler-delta-signatures/`). It is the handoff's queue restated as
  current inside a spec, so success criterion 1 owns it; it is not on `docs/TODO.md`'s
  stale-citation list, so phase 2 would not have caught it.
- 2026-09-04 (plan 01): `.claude/active-plan` staleness recorded under Out of scope rather than
  given a phase row — see the rationale there.
- 2026-09-04 (plan 02): phase 2's citation sites include five Rust files (`metadata.rs`,
  `rules/incremental.rs`, `gate.rs`, `propagate.rs`, `propagation.rs`, `refresh_axis.rs`). The
  outcome's "no crate changes" rule is read as *no behaviour change*: doc-comment-only edits are in
  scope, since criterion 3 names those sites explicitly and they cannot be closed otherwise.
- 2026-09-04 (plan 02): the TODO's inventory is slightly wrong — its "one e2e test" site is
  actually production `crates/smelt-core/src/metadata.rs`, and `rules/incremental.rs:708` has moved
  to :457. Phase 2 resolves sites by *heading text*, not by the TODO's line numbers.
- 2026-09-04 (plan 02): `docs/plans/` and `docs/research/` also cite the dead headings; they stay
  untouched as historical records and are not added as a phase row.
- 2026-09-04 (implement 01): phase 1 done — banner added to the 2026-08-16 handoff,
  `run_state.md` re-pointed, backlog order confirmed unchanged, `verify-phase.sh` green. The
  dangling-outcome sweep's literal directory-arg form also flags `.claude/usage-log.jsonl` (a
  gitignored local log); restricting to `git ls-files` is the correct read and is clean.
- 2026-09-04 (implement 02): phase 2 done — all eight sites re-pointed or dropped; site 4's
  plan-suggested target (§"Batch safety classification") didn't actually carry the claim, so the
  real target (§"Safety checks (per-cell admission for recompute-a-region)") was used instead
  after reading both. A new gap surfaced and was recorded as a fresh `docs/TODO.md` bullet rather
  than fixed: `§"What the composed shape uniquely enables"` is cited from three sites but doesn't
  exist in either incremental spec; out of this phase's site list, so left for a future sweep.
- 2026-09-04 (plan 03): phase 3 requires three normative calls the outcome did not pre-decide;
  made here rather than blocked, because `state.md` §"The optionality rule" constrains each to one
  of two shapes and each reuses a diagnostic `state.md` §Surface already owns. Schema snapshots →
  degrade (every model reads as `new`, ordinary create/replace, never `NoChange`); source postures
  → degrade (cross-run-baseline verification cannot run, so the narrowing declaration's fold
  licence is withheld to the undeclared row, recorded `MaintenanceStateDowngraded`; scan-window
  probes unaffected); frozen-band probe baselines → refuse by name
  (`DeclaredContractRequiresState`, the same call `state.md` makes for `contract.deferral`).
- 2026-09-04 (plan 03): no phase reshape — the phase-02 summary's one open item (the dangling
  `§"What the composed shape uniquely enables"` citations) is outside this outcome's success
  criteria and is already recorded as a fresh `docs/TODO.md` bullet.
- 2026-09-04 (plan 04): phase 4 widened (not reshaped) to include
  `docs-site/docs/guide/incremental-models.md` §"Enrichment joins and dimension updates", whose
  `smelt explain daily_events_enriched` transcript prints `ColumnScopedMerge` for the `{user_name}`
  cell that model does not derive. It is the identical false claim about the identical model, one
  layer up, so leaving it would defeat criterion 5's point (one consistent record) while satisfying
  its letter. The docs-site delta-signature pass stays out of scope as declared.
- 2026-09-04 (plan 04): technique confirmed empirically at plan time —
  `smelt explain daily_events_enriched` reports `RecomputeRegion`/`DeleteInsert`/`WholeRow` for all
  four cells. No pinning cargo test is added: the outcome is docs-only and this fixture's technique
  is already the subject of `docs/TODO.md`'s reachability entry, so a test here would duplicate a
  tracked gap rather than close one.
- 2026-09-04 (implement 03): phase 3 done — all three owner-spec sentences landed verbatim as
  the plan specified, the `state.md` divergence bullet deleted, all five `rg` checks green,
  `verify-phase.sh` green. No new gaps surfaced.
- 2026-09-04 (implement 04): phase 4 done — fixture and docs-site comments corrected to the
  derived `DeleteInsert` technique, TODO note removed, `verify-phase.sh` green. Found (not
  fixed, out of this phase's task list) that `daily_events_status.sql` and `user_status.yml`
  carry the identical overclaim for a different model; recorded as a fresh `docs/TODO.md`
  bullet for a future phase or outcome.
- 2026-09-04 (plan 05): phase table reshaped — a new phase 5 added for the sibling overclaim the
  phase-04 summary surfaced (`daily_events_status.sql`, `models/sources/raw/user_status.yml`),
  validation moved to phase 6. It is the identical false technique claim in the identical example
  workspace as criterion 5's fixture, and the outcome's own statement is "one consistent record":
  leaving a known-false `ColumnScopedMerge` claim next to the one just corrected satisfies
  criterion 5's letter while defeating its point. Same reasoning plan 04 used to widen into
  docs-site. Docs-only, and the correct technique is already established empirically.
- 2026-09-04 (plan 05): confirmed at plan time via
  `smelt explain daily_events_status --project-dir examples/timeseries` — both `UpstreamMutation`
  cells derive `RecomputeRegion`/`DeleteInsert`, but their `partition_local` locality and the
  `raw.user_status`/`changed_at` `ScanClamp` **are** real. Only the MP11/F15 column-scoped-`MERGE`
  sentences are false; the clocked-dimension / `PartitionLocal::Yes` contrast with
  `daily_events_enriched.sql` must be preserved, since it is the reason this fixture exists.

- 2026-09-04 (implement 05): phase 5 done — `daily_events_status.sql` and
  `user_status.yml` comments corrected to the derived `RecomputeRegion`/`DeleteInsert`
  technique, the `ColumnScopedMerge` overclaim removed, `docs/TODO.md` bullet deleted.
  Ground truth matched the plan exactly; no new gaps surfaced.

- 2026-09-04 (plan 06): no reshape — the phase-05 summary surfaced no new gaps, and its one
  carried-over item (the dangling `§"What the composed shape uniquely enables"` citations) is
  already recorded as a `docs/TODO.md` bullet and sits outside this outcome's success criteria.
- 2026-09-04 (plan 06): criterion 6 is read as "no drift *introduced by this outcome*", per its
  own wording. Phase 6 therefore classifies every validate finding as outcome-introduced (fix it,
  since it is this outcome's own damage) or pre-existing (record in `docs/TODO.md`, do not fix —
  fixing it would be the code/spec work this outcome puts out of scope).

- 2026-09-04 (implement 06): phase 6 done — both `/smelt:validate` runs clean, all six
  criterion `rg` checks PASS, `verify-phase.sh` ALL GREEN, `example_diagnostics` and
  `explain_model` green. One pre-existing (not outcome-introduced) freshness flag found on
  `state.md` vs. an unrelated 2026-09-04 commit; recorded as a new `docs/TODO.md` bullet rather
  than fixed, per the outcome's docs-only/no-crate-changes scope. **All six success criteria
  are now met — this outcome is closed.**

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
