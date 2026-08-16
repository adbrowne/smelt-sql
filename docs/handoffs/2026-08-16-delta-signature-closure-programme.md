# Handoff: the delta-signature closure programme (2026-08-16)

**For:** a fresh session picking up the replanning of the incremental-spec implementation work.
**Branch context:** written on `spec-redraft-incremental-models` (worktree `incremental3`), 308 commits ahead of `main`, main has nothing this branch lacks.
**Status:** programme design agreed with Andrew in-session; **nothing executed yet**. The concrete actions in §Immediate actions are all still to do.

## The decision

Every previously queued plan/outcome is **replaced** by this programme. Nothing in
`docs/plans/` or the four queued `docs/outcomes/20260815-*` directories will ever be run
as written. The new programme below is all there is.

Andrew's three shaping answers (asked and answered 2026-08-16):

1. **Open Questions get a decision track** — an interactive session (Andrew + Claude, not
   the loop) triages the ~20 `(Open Question)` bullets across the specs; decided ones land
   as spec diffs first, then graduate into the residue outcomes. The loop never resolves
   product questions unilaterally.
2. **Merge to main before the new programme starts.** PR `spec-redraft-incremental-models`
   → `main`; the programme then runs on a fresh branch/worktree off post-merge main.
3. **Retire superseded docs in place** — queued outcomes get `Status: superseded` plus a
   pointer here; done outcomes and historical plans are untouched (plans-are-historical rule).

## Situation assessment (why replan)

- The 2026-08-09 outcome programme is **fully done** (six outcomes: rung2-state-shapes,
  repair-family, probe-backed-facts, output-delta-typing, contract-lattice-v1,
  incremental-spec-redraft). The maintenance machinery largely exists.
- The redraft split the spec into three (`incremental_models.md` shared layer,
  `incremental_shapes.md` shape profiles, `definition_deltas.md`) plus new `state.md`
  (state-ownership doctrine) and re-pointed `run_state.md`. Their **Known Divergences
  sections are gap-first lists and are the authoritative statement of the implementation
  gap** — roughly 70 bullets across the five specs. Do not re-derive the gap; read those
  sections.
- The queued 2026-08-15 programme was rejected because it funnelled almost everything into
  one 20-phase mega-outcome (`20260815-definition-delta-migrate`, widened to "close every
  residue everywhere") — a grab-bag, not foundations-first. Its own history shows the
  churn: scope widened to build-everything (`725898bf`) and reversed the same day
  (`e1b541e5`).

## Gap clusters (what the ~70 bullets reduce to)

1. **State ownership** — `state.md` entirely unimplemented: `state.mode` parsed but never
   consulted; the reconciliation ledger lives in `.smelt/` violating the residency rule
   (the spec's own "flagship gap": deleting `.smelt/` can corrupt keyed additive folds);
   the two-step ideal-then-availability-resolution derivation and
   `MaintenanceStateDowngraded` don't exist.
2. **Scheduler doesn't consume delta signatures** — run-loop currency is still whole
   day-intervals; key-addressed repair cells derived but never dispatched outside the
   `grain: key` branch; keyed dirt carries key *columns*, not values; `--since-upstream`
   doesn't read recorded deltas live; no persisted per-source watermark. Divergence #1 in
   `incremental_models.md`.
3. **Definition-delta vertical** — `crates/smelt-logical/src/backbuild/` is tested dead
   code; `smelt migrate`, the plan-hash approval store, the `backbuild`→`rebuild` rename,
   `MaintenanceSkeletonChanged` rename, and the conformance definition-edit step kind are
   all unwired.
4. **Observability parity** — `smelt explain` doesn't print the delta-signature headline,
   per-column guarantees, or derived run shape; several refusals lack pre-execution
   surfacing.
5. **Shape residues** — the keyed-grain and partition-grain bullet lists (content in the
   superseded `20260815-keyed-grain-residue` / `20260815-partition-grain-residue`
   outcome docs is sound and reusable).

## The programme

Sequenced outcomes, one coherent capability each, success criteria drawn verbatim from the
Known Divergences bullets each closes, conformance-gate-backed, scaffolded via
`/smelt:outcome`, run by the outcome loop:

1. **`state-residency`** — implement `state.mode`; move the reconciliation ledger
   engine-resident; build availability resolution with `MaintenanceStateDowngraded` +
   `DeclaredContractRequiresState`; add the state-deletion conformance leg (only possible
   after the ledger move — the reason this is first: everything keyed writes through the
   frontier, and per-cell deferral scheduling + non-DuckDB ledgers need it in its final
   home).
2. **`scheduler-delta-signatures`** — run loop dispatches typed delta components:
   key-addressed cells outside the `grain: key` branch, key-valued dirt-sets, live
   observed-delta consumption, persisted per-source watermark; `smelt explain` prints the
   signature headline / per-column guarantees / derived run shape as the verification
   surface. **Highest design risk in the programme** — Andrew should review its first
   Opus-planned phase before the loop executes it.
3. **`definition-delta-migrate-v2`** — the narrow vertical only (the superseded outcome's
   original phases 2–9): wire `backbuild/` into `smelt migrate` (plan-only, `--apply`,
   plan-hash approval store, CI exit codes), `backbuild`→`rebuild` rename,
   `MaintenanceSkeletonChanged` rename sweep across code + sibling specs, conformance
   definition-edit step kind, docs-site migration guide.
4. **`keyed-grain-residue-v2`** and 5. **`partition-grain-residue-v2`** — carried content
   plus whatever the decision track graduates. **Do not scaffold until at least one
   decision-track session has happened** (decisions will grow their scope).
6. **`closure-confirm-v2`** — re-baselined audit: every Known Divergences bullet across
   all five specs checked against the repo; closed-means-closed, open-means-accurately-open;
   unresolved residue reopens the owning outcome.

**Decision track** (parallel to outcomes 1–3, interactive): triage the ~20 Open Questions
batched by theme — key deletion/retention, change-feed consumption, `g_run ≥ g_part`
auto-coarsening, override-ladder/cost-model reach, out-of-band-edit tripwire, `state.mode`
warehouse-bookkeeping opt-out, snapshot-reconcile multi-source, run-pinning, driver
granularities, self-referential keyed models. Each decision is a spec diff first
(spec-first rule); undecided ones stay honestly open. §Future Extensions of
`incremental_models.md` gives a priority order for lattice-point work (retention/key
departure first) if the triage wants a starting order.

## Immediate actions (none done yet)

1. Open the merge PR `spec-redraft-incremental-models` → `main`; let standing gates run;
   merge.
2. Fresh worktree/branch off post-merge main for the programme.
3. Retire the four queued outcomes in place (`Status: superseded` + pointer to this
   handoff); rewrite `.claude/outcome-backlog` to the new queue (outcomes 1–3 only for
   now).
4. Scaffold outcomes 1–3 via `/smelt:outcome`.
5. Launch the outcome loop (per `docs/outcome_loop.md`; detached tmux/systemd, never a
   backgrounded Bash call; export `DUCKDB_LIB_DIR` + `LD_LIBRARY_PATH` into the loop env).
6. Schedule the first decision-track session with Andrew.

## Pointers

- Gap lists: §Known Divergences of `docs/specs/incremental_models.md` (25 items),
  `docs/specs/incremental_shapes.md` (10 partition + 19 keyed), `docs/specs/definition_deltas.md`
  (6), `docs/specs/state.md` (5), `docs/specs/run_state.md` (7).
- Superseded queue (content still useful): `docs/outcomes/20260815-*/outcome.md`.
- Done programme (context for what exists): `docs/outcomes/20260809-*/`.
- Outcome-loop operator guide: `docs/outcome_loop.md`; scaffolder: `/smelt:outcome`.
- Also stale and superseded by this programme: the `.claude/active-plan` pointer (still
  names the 20260719 production-readiness programme from the worktree-production branch)
  — do not resume it from this worktree.
