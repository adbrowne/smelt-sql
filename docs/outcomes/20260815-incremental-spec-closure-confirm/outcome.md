# Outcome: Confirm every closeable-without-design divergence is closed, and everything left is honestly flagged

**Created:** 2026-08-15
**Status:** superseded
**Superseded by:** the delta-signature closure programme — `docs/handoffs/2026-08-16-delta-signature-closure-programme.md`. This outcome will never be run as written; its content remains reusable by the successor outcomes named there.
**Source:** `docs/outcomes/20260815-definition-delta-migrate` and the two outcomes it spawned
(`keyed-grain-residue`, `partition-grain-residue`)
**Spec anchors:** `docs/specs/definition_deltas.md`, `docs/specs/incremental_models.md`,
`docs/specs/incremental_shapes.md`, `docs/specs/model_properties.md`

## The outcome

This is the closing audit for the 2026-08-15 program, run only once `definition-delta-migrate`,
`keyed-grain-residue`, and `partition-grain-residue` all reach `done`. It does not implement
anything new and does **not** claim zero remaining `(Open Question)` tags — several were
deliberately left open because closing them means choosing new admission width or new surface that
this program declined to decide unilaterally (`docs/outcomes/20260815-definition-delta-migrate`
§"Out of scope"). What it does confirm, with a checkable artifact: every `§Known Divergences`
bullet that was closeable *without* a fresh product decision is actually closed (verified against
the repo, not an outcome's own say-so), and every bullet left open is still accurately described in
the spec as an open, undecided question — none of them silently reads as settled, and none of them
went stale (still-live bullets that got fixed as a side effect of other work, but never had their
divergence entry removed).

## Success criteria (checkable)

1. A written closure report (`closure-report.md` in this outcome's directory) enumerates every
   `§Known Divergences` bullet and `(Open Question)` tag that existed across the four anchor specs
   at the 2026-08-15 baseline (reconstructed from git history), and states each one's disposition:
   closed (with the commit), or still open with the reason it needs a product decision this
   program didn't make.
2. Every bullet `docs/outcomes/20260815-keyed-grain-residue` and
   `docs/outcomes/20260815-partition-grain-residue` claim to close is actually removed from
   `incremental_shapes.md` §Known Divergences, not just addressed in code.
3. Every bullet still named in `docs/outcomes/20260815-definition-delta-migrate` §"Out of scope"
   as needing sign-off is spot-checked against the current spec text: still accurately worded,
   still tagged `(Open Question)` where it should be, and not accidentally fixed by unrelated work
   without its divergence entry being removed (a bullet whose underlying behaviour changed but
   whose spec text wasn't updated is itself a drift bug to report and fix here).
4. `/smelt:validate definition_deltas`, `/smelt:validate incremental_models`, `/smelt:validate
   incremental_shapes`, and `/smelt:validate model_properties` all report no drift.
5. The full standing-gate suite is green: `bash .claude/scripts/verify-phase.sh`,
   `cargo test -p smelt-cli --test maintenance_conformance`,
   `cargo test -p smelt-runtime --test statement_parity`,
   `cargo test -p smelt-logical --test walk_coverage`,
   `cargo test -p smelt-runtime --test execute_parity`.
6. If any bullet claimed-closed by an owning outcome has no clean disposition (the outcome shipped
   something narrower than claimed, or got blocked), that residue is named explicitly in the
   closure report and this outcome does **not** claim completion until it's resolved — reopen the
   owning outcome, don't paper over it here.

## Out of scope

- Deciding any of the still-open `(Open Question)` bullets. That's the explicit boundary this
  whole program drew; this outcome's job is to confirm the boundary is honestly documented, not to
  cross it.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Reconstruct the 2026-08-15 baseline bullet inventory (Known Divergences + Open Questions) across all four specs from git history | pending |
| 2 | Cross-check every baseline bullet against the current repo state and each owning outcome's decision log; classify closed / still-open-and-accurate / drifted | pending |
| 3 | Resolve any residual or drifted bullets: reopen the owning outcome, or fix the stale spec wording directly if trivial | pending |
| 4 | Run all four `/smelt:validate` invocations; fix any drift | pending |
| 5 | Write `closure-report.md`; final standing-gate run | pending |

## Decision log

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
