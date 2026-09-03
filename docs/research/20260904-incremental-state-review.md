# Incremental-model state review (2026-09-04)

A review of where the incremental-models work stands on `spec-redraft-incremental-models` at
the close of the 2026-08-28 → 2026-09-04 outcome-loop burst, an assessment of the outcome
programme that produced it, and a recommended next sequence. Written at the request of
Andrew before merging PR #185.

## Where the branch stands

PR #185 (`spec-redraft-incremental-models` → `main`) is mergeable with every CI check green,
including the Spark Delta parity and maintenance-conformance-twin legs, and has `main` fully
merged in.

| | |
|---|---|
| Commits ahead of `main` | 149 |
| Diff | 450 files, +52.6k / −3.6k |
| Crate lines, production vs test-ish | ~17k vs ~21k |
| Outcomes run | 4 (3 `done`, 1 `blocked`) |
| Baseline divergence bullets audited | 80: 35 closed, 29 open, 16 reworded, 0 false closures |

What shipped in the burst:

- `smelt migrate` with plan, approval store, `--apply`, `--json` and CI exit codes; the
  `backbuild` → `rebuild` rename; definition-edit steps in the generative conformance suite;
  `MaintenanceSkeletonChanged` reaching LSP diagnostics via a Salsa world-fact input.
- Posture-derived key deletion at snapshot reconcile with the `retain_departed` lattice point
  (declaration, oracle transform, probe emitter, manifest record).
- Graph-layer scheduler pieces: key-addressed cell dispatch outside the `grain: key` branch,
  keyed dirt propagation, time-unrolled self-edges, `--select` scoping for `--since-upstream`,
  observed-delta read and write sides, hour-granularity propagation.
- Execution postures derived once in `smelt-logical` and printed by `smelt explain`.
- The typed integer partition axis with an end-to-end run; CTE-hidden `event_time_column`
  refusal; partition-grain classification through expanded function bodies.
- The nullable-payload conformance pool proving the once-write NULL direction; the gated
  Spark/BigQuery conformance twin un-rotted with a per-PR compile guard.

A spot-check of the departed-key delete commit (`5f3327bd`) showed careful runtime code with
a statement-parity extension and a dedicated end-to-end test, not scaffolding.

## Assessment of the outcome programme

### What held up

- **Honesty discipline.** `20260815-incremental-spec-closure-confirm` is the strongest artifact
  of the set: it reconstructed the 80-bullet baseline from git, verified every closure claim
  against the repo rather than the owning outcome's say-so, caught and corrected its own
  IS-24/IS-18 mislabel, and correctly refused to count the blocked keyed-grain phase as a
  false closure.
- **Out-of-scope findings were recorded, not dropped.** 68 phase summaries carry a
  "for the next planner" section. Two genuine bugs found this way were fixed in-programme:
  the `order_id` keyword collision in `group_by_unique_key` (phase 13 → 17) and the
  double-DuckDB-connection write loss in `run_since_upstream` (phase 15).
- **Fail-loud was applied to the programme's own residue.** The silent `tracing::warn` skip
  phase 2 of the keyed-grain outcome introduced became a reported `RunReporter` event in
  phase 7 rather than being deferred with the blocked phase.

### What went wrong

- **The programme ran the plan that had already been rejected.** The handoff
  `docs/handoffs/2026-08-16-delta-signature-closure-programme.md` states that every queued
  `20260815-*` outcome is replaced and "will ever be run as written", and lays out a
  foundations-first sequence: `state-residency` first, `scheduler-delta-signatures` second, a
  narrow `definition-delta-migrate-v2` third. Nothing in the repo records that decision being
  reversed. What ran instead was the outcome the handoff called "a grab-bag, not
  foundations-first", grown from 20 to 34 phases. Some of the scheduler cluster was absorbed
  into it (phases 11–24b), which is sound work, but the ordering rationale was lost.
- **Skipping `state-residency` is now the visible blocker.** `20260815-keyed-grain-residue`
  phase 3 is blocked on exactly what the handoff put first: no availability-resolution step,
  no `MaintenanceStateDowngraded`, and a reconciliation ledger still resident in `.smelt/`.
  `state.md`'s flagship gap is therefore still open: deleting `.smelt/` can still corrupt a
  keyed additive fold.
- **The closure audit has a blind spot.** Its denominator was the four anchor specs
  (`definition_deltas`, `incremental_models`, `incremental_shapes`, `model_properties`). The
  handoff's gap list also named `state.md` (5 bullets) and `run_state.md` (7). All five
  `state.md` bullets are closeable without a product decision and none was audited, so
  "everything closeable is closed" does not hold for the state cluster.
- **Ratchets crept without sign-off.** `.claude/hardening-baseline.txt` moved from
  `smelt-cli println 161 → 172` and `smelt-db unwrap 16 → 19` during the burst with no
  reviewer note in the commits, which the CLAUDE.md rule asks for. `execute.rs` and
  `maintenance_driver.rs` are now each over 6,000 lines. Neither is a defect; both are the
  kind of drift a 34-phase Sonnet-implemented outcome accumulates without anyone deciding to.
- **The docs-site lags the spec's front door.** The specs were re-architected around delta
  signatures on 2026-08-12; the word "signature" does not appear in
  `docs-site/docs/guide/incremental-models.md`. `docs/TODO.md` already records this as
  deferred. Every other user-facing surface from the burst did get docs.

## Recommended next sequence

1. **Merge PR #185.** Green, honest about its gaps, and 149 commits deep invites conflict.
2. **Decide keyed-grain phase 3.** Its blocked entry offers three options. Recommended:
   option 1, amend the criterion to the already-recorded decision (a ledger-less backend
   takes a recorded, explain-visible downgrade). That is `state-residency` under another
   name and should be its own outcome, not a row in the residue outcome.
3. **Run `state-residency` next, as the handoff intended.** Move the ledger engine-resident,
   implement `state.mode` and availability resolution with `MaintenanceStateDowngraded` /
   `DeclaredContractRequiresState`, add the state-deletion conformance leg. This is the one
   cluster where the spec is fully decided and the code is entirely absent, and it closes
   the correctness flagship gap.
4. **Hold a short decision-track session before scaffolding anything else.** The closure
   report already splits the 29 open bullets into plain unscheduled backlog and genuine
   product calls. Deciding three or four of the latter (`PartitionGrainForbidsMetrics`, the
   determinism scope, the sub-`g_part` suggestion text) would let a `residue-v2` outcome
   absorb them cheaply.
5. **Reconcile the handoff with what happened.** Either mark
   `2026-08-16-delta-signature-closure-programme.md` superseded with a pointer here, or
   restore its sequence in `.claude/outcome-backlog`. A fresh session reads the handoff first
   and today finds two contradictory statements of the programme.
6. **Docs-site pass on the delta-signature front door**, sequenced after `smelt explain`
   prints the signature headline, so the docs describe output the user can see.

Do not start `scheduler-delta-signatures` yet. The handoff flagged it as the highest
design-risk item and asked for a human review of its first plan, and the pieces the
mega-outcome already landed change what that plan should say.

## Pointers

- Closure audit: `docs/outcomes/20260815-incremental-spec-closure-confirm/closure-report.md`.
- Blocked decision: `docs/outcomes/20260815-keyed-grain-residue/outcome.md` §Blocked.
- Superseded-then-run programme: `docs/handoffs/2026-08-16-delta-signature-closure-programme.md`.
- State gap list: `docs/specs/state.md` §Known Divergences.
- Open-question decisions: `docs/research/20260816-open-questions-triage.md`.
