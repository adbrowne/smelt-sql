# Outcome: Closure-confirm audit (v2, re-baselined)

**Created:** 2026-08-16
**Status:** queued
**Source:** `docs/handoffs/2026-08-16-delta-signature-closure-programme.md` (programme outcome 6);
supersedes `docs/outcomes/20260815-incremental-spec-closure-confirm/` (re-baselined against the
post-decision-track specs and the v2 outcome set).
**Spec anchors:** `docs/specs/incremental_models.md`, `docs/specs/incremental_shapes.md`,
`docs/specs/definition_deltas.md`, `docs/specs/state.md`, `docs/specs/run_state.md`

## The outcome

The programme's final audit: every Known Divergences bullet across the five incremental-family
specs is checked against the repository as it actually stands — closed means closed (the bullet
is gone and the behaviour verifiably shipped), open means accurately open (the bullet's wording
still matches reality). Every bullet a programme outcome claimed to close is verified shipped;
any unresolved residue reopens the owning outcome rather than being papered over. The audit also
confirms the decision track's spec diffs still match the shipped surface (no drift snuck in
during implementation) and that the deferred items still sit honestly in Future Extensions.

## Success criteria (checkable)

1. Every §Known Divergences bullet in the five specs is dispositioned in an audit table:
   **closed** (behaviour shipped, bullet removed, verifying test/gate named) or **open**
   (bullet wording still accurate today). No bullet is left claiming a state that isn't real.
2. Every bullet claimed closed by `20260816-state-residency`, `-scheduler-delta-signatures`,
   `-definition-delta-migrate-v2`, `-keyed-grain-residue-v2`, and `-partition-grain-residue-v2`
   is verified against the repo; any shortfall reopens the owning outcome (recorded in its
   phase table), never silently dropped.
3. `/smelt:validate` runs clean for `incremental_models`, `incremental_shapes`,
   `definition_deltas`, `state`, and `run_state` (or every reported drift is a recorded,
   accurately-worded divergence).
4. The decision record (`docs/research/20260816-open-questions-triage.md`) is cross-checked:
   each decided item's spec text still matches what shipped; each deferred item still sits in
   Future Extensions, undecided and unrelied-upon.
5. All standing gates green; the audit table is committed under this outcome's directory.

## Out of scope

- Closing any gap itself — this outcome only audits and reopens; implementation belongs to the
  owning outcome.
- Specs outside the incremental family (their divergence lists are other programmes' concerns,
  except bullets the programme outcomes explicitly swept).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Build the audit table: every Known Divergences bullet across the five specs, dispositioned closed/open with evidence | pending |
| 2 | Verify every programme-outcome-claimed closure against the repo; reopen owning outcomes for any shortfall | pending |
| 3 | `/smelt:validate` sweep across the five specs; reconcile every reported drift | pending |
| 4 | Cross-check the decision record against shipped surface and Future Extensions honesty | pending |
| 5 | Close out: commit the audit, final gate sweep, programme summary for the handoff trail | pending |

## Decision log

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
