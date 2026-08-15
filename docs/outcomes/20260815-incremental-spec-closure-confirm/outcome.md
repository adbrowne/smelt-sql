# Outcome: Confirm zero open questions / known divergences across the incremental spec family

**Created:** 2026-08-15
**Status:** queued
**Source:** the full 2026-08-15 incremental-spec build-out — `docs/outcomes/20260815-definition-delta-migrate`
and the nine outcomes it spawned (`keyed-open-questions-buildout`, `partition-grain-residue`,
`key-locality-and-deletion`, `ladder-rungs-3-4`, `lattice-v2`, `proofs-as-product`,
`scd2-watermark-observer-contract`, `retraction-and-changefeed`,
`refresh-mode-consolidation-audit`)
**Spec anchors:** `docs/specs/definition_deltas.md`, `docs/specs/incremental_models.md`,
`docs/specs/incremental_shapes.md`, `docs/specs/model_properties.md`

## The outcome

This is the closing audit for the whole 2026-08-15 program, run only once every outcome listed
above (and the migrate outcome itself) reaches `done`. It does not implement anything new — it
confirms, with a checkable artifact, that `definition_deltas.md`, `incremental_models.md`,
`incremental_shapes.md`, and `model_properties.md` carry zero `§Known Divergences` bullets and zero
`(Open Question)` tags left over from the 2026-08-15 baseline. Every bullet that existed at that
baseline is accounted for: either its owning outcome closed it (bullet removed, verified against
the repo not the outcome's own say-so), or it was deliberately reclassified into `§Future
Extensions` as accepted forward work with no divergence framing — never silently dropped, never
left half-true.

## Success criteria (checkable)

1. A written closure report (`closure-report.md` in this outcome's directory) enumerates every
   `§Known Divergences` bullet and `(Open Question)` tag that existed across the four anchor specs
   at the 2026-08-15 baseline (reconstructed from git history at commit `6cef4627` or the nearest
   available baseline), and states each one's disposition: closed by outcome `<name>` (with the
   commit that removed it), or reclassified to `§Future Extensions` (with the rationale).
2. `rg -n '\(Open Question\)'` across all four spec files returns nothing.
3. Each spec's `§Known Divergences` section is either absent, empty, or contains only bullets
   created *after* the 2026-08-15 baseline (a regression introduced by this program's own
   implementation work must be closed here too, not waved through).
4. `/smelt:validate definition_deltas`, `/smelt:validate incremental_models`, `/smelt:validate
   incremental_shapes`, and `/smelt:validate model_properties` all report no drift.
5. The full standing-gate suite is green: `bash .claude/scripts/verify-phase.sh`,
   `cargo test -p smelt-cli --test maintenance_conformance`,
   `cargo test -p smelt-runtime --test statement_parity`,
   `cargo test -p smelt-logical --test walk_coverage`,
   `cargo test -p smelt-runtime --test execute_parity`.
6. If any bullet from the baseline inventory has no clean disposition (its owning outcome shipped
   something narrower than the bullet, or got blocked), that residue is named explicitly in the
   closure report and this outcome does **not** claim completion until it's resolved one way or
   the other — a residue is a finding to act on, not something to paper over with prose.

## Out of scope

- Nothing. This outcome's entire job is confirming the other nine (plus the migrate outcome)
  actually closed what they claimed; if it finds a gap, the fix belongs to the owning outcome
  (reopen it) or, for a genuinely new finding, a fresh outcome queued behind this one — not a
  quiet edit inside this closure outcome's own phases.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Reconstruct the 2026-08-15 baseline bullet inventory (Known Divergences + Open Questions) across all four specs from git history | pending |
| 2 | Cross-check every baseline bullet against the current repo state and each owning outcome's decision log; classify closed / reclassified / residual | pending |
| 3 | Resolve any residual bullets: reopen the owning outcome, or queue a new outcome, or (rarely) fix directly if truly trivial | pending |
| 4 | Run all four `/smelt:validate` invocations; fix any drift | pending |
| 5 | Write `closure-report.md`; final `rg '(Open Question)'` sweep and standing-gate run | pending |

## Decision log

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
