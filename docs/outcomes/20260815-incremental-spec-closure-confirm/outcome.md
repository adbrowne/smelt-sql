# Outcome: Confirm every closeable-without-design divergence is closed, and everything left is honestly flagged

**Created:** 2026-08-15
**Status:** active
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
| 1 | Reconstruct the 2026-08-15 baseline bullet inventory (Known Divergences + Open Questions) across all four specs from git history | done |
| 2 | Cross-check every baseline bullet against the current repo state and each owning outcome's decision log; classify closed / still-open-and-accurate / drifted | done |
| 3 | Resolve residual/drifted bullets and spot-check the `definition-delta-migrate` §Out-of-scope bullets: fix stale spec wording directly if trivial, otherwise open a row or a Blocked entry | done |
| 4 | Run all four `/smelt:validate` invocations; fix any drift | planned |
| 5 | Write `closure-report.md`; final standing-gate run | pending |

## Decision log

- 2026-09-04 — Phase 1 planned. Two facts fixed here. (a) The program baseline is commit
  `03a431f3` (`outcome(20260815-definition-delta-migrate): scaffold`), the first commit of the
  2026-08-15 program; every later spec edit is in scope for classification. (b) The precondition
  in §"The outcome" — run only once all three owning outcomes are `done` — is **not** met:
  `20260815-keyed-grain-residue` is `**Status:** blocked` (its phase 3, "Transactional ledger fold
  on every shipped backend"; all its other rows are `done`). This does not block the audit — it is
  precisely the case success criterion 6 exists for — so the audit proceeds and phase 3's row is
  reworded to name that residue explicitly. No other reshape: rows 2-5 still map one-for-one onto
  criteria 2-3, 4 and 1/5.
- 2026-09-04 — Phase 1 done. 80 bullets extracted from the four anchor specs at commit
  `03a431f3` (`definition_deltas` 7, `incremental_models` 25, `incremental_shapes` 32,
  `model_properties` 16 — all match the plan's sample). The extractor's Open-Question count
  disagrees with the plan's sample for two specs (`incremental_models` 7 not 6,
  `incremental_shapes` 16 not 13): 5 bullets wrap `(Open`/`Question)` across a markdown line
  break, which a naive single-line grep misses but the whitespace-collapsing extractor catches.
  Per plan instruction, the extractor's count is authoritative; documented in
  `baseline-inventory.md` §Extraction notes. `incremental_shapes` `IS-24` (the transactional-fold
  bullet) is flagged for phase 2 as the `keyed-grain-residue` blocked-phase bullet — not to be
  marked closed without independent repo-state verification.

- 2026-09-04 — Phase 2 planned. No reshape: phase 1 surfaced no work needing a new row (its one
  finding, the corrected Open-Question denominators, is already carried in `baseline-inventory.md`
  and lands in the phase-5 report), and row 3 already names the `IS-24` / `keyed-grain-residue`
  residue that phase 1 flagged. Fixed here: the four-value disposition vocabulary
  (`closed <sha>` / `open` / `drifted` / `residue`) and the rule that a `closed` claim is verified
  against the repo, never against the removing commit's message or an owning outcome's say-so.
- 2026-09-04 — Phase 2 done. All 80 baseline bullets classified: 35 `closed`, 29 `open`, 16
  `drifted`, **0 `residue`** — every owning-outcome closure claim independently confirmed against
  code/tests/a landed decision record. Found and fixed a mislabeling carried from phase 1's
  decision log: the transactional-merge-ledger bullet (the one `20260815-keyed-grain-residue`
  phase 3 is blocked on) is `IS-18`, not `IS-24` as previously written here — `IS-24` is a
  different bullet (recurrence-bound slice pruning / granularity relaxation / slice-scoped
  deletion), now `drifted` (moved to §Future Extensions) for unrelated reasons. `IS-18` itself
  classified `drifted` (bold lead-in reworded, same DuckDB-only gap), not `residue`: the blocked
  outcome's decision log honestly states the criterion is "deliberately left unmet" and never
  claims closure, so success criterion 6 doesn't fire — no owning-outcome reopen is needed. Full
  detail in `phases/02-summary.md` and `baseline-inventory.md` §Classification summary. Row 3 is
  lighter-scoped than anticipated (a wording spot-check on the 16 `drifted` rows, not a reopen).

- 2026-09-04 — Phase 3 planned. Light reshape, no new rows. (a) Row 3's wording is updated: phase 2
  found **zero `residue`** and confirmed the `keyed-grain-residue` blocked bullet (`IS-18`) never
  claimed closure, so there is no owning-outcome reopen to do — the row's real content is the
  `drifted` wording spot-check. (b) Row 3 now explicitly absorbs success criterion 3's
  out-of-scope spot-check (every bullet `20260815-definition-delta-migrate` §"Out of scope" names:
  still present, still `(Open Question)`-tagged where claimed, behaviour still missing), which was
  previously implicit across rows 2-3 and which phase 2 did not cover — phase 2 classified the 80
  *baseline* bullets, while the out-of-scope list also names §Future Extensions material that is
  not a baseline Known-Divergence row. (c) Standing rule fixed for this phase: a stale bullet is
  edited inline only when the fix is wording; a stale bullet needing implementation gets a new
  phase row, and one needing a product decision gets a `## Blocked` entry — the audit never
  invents a decision the program declined to make.

- 2026-09-04 — Phase 3 done. All 16 `drifted` rows given a phase-3 Verdict: 11 `accurate`
  (reworded but still describe a real, still-open gap), 5 `relocated` (moved to `§Future
  Extensions`, still honestly framed as undecided-future). Zero `stale-fixed` — no spec text
  needed editing, so this phase produced no spec diff. `check-classification.sh` extended to
  enforce the Verdict column (confirmed red on all 16 rows before filling it in, green after).
  The `20260815-definition-delta-migrate` §"Out of scope" sweep (success criterion 3) added a new
  `baseline-inventory.md` §"Out-of-scope spot-check" table, one row per named item: every item's
  spec/section still exists and is still honestly framed, with one exception that is **not** a
  spec-text divergence — the out-of-scope prose's own claim that `docs/plans/20260704-model-
  updates.md`'s D1–D3 fate is "unclear individually" is stale for the D3 leg
  (`refresh: materialized_view` has since shipped as fully-specified surface,
  `docs/specs/materialized_view.md`); D1/D2 (`latest_value`/`versioned`) remain genuinely absent
  and unclear. No spec bullet claims D3 undecided, so nothing to fix under the spec-only-if-
  trivial rule, and no product decision is needed (D3 is already decided and shipped) — recorded
  as a closure-report footnote per the standing rule, not a new row or a `## Blocked` entry. The
  `IS-24`/`IS-18` mislabel cross-check (task 7) came back clean in both owning outcomes' files.

- 2026-09-04 — Phase 4 planned. No reshape: phase 3 produced no spec diff, added no row and no
  `## Blocked` entry, so rows 4-5 stand as written (criterion 4, then criteria 1/5). Two things
  fixed for this phase. (a) The drift-disposition rule is widened one level from phase 3's:
  doc/wording drift is fixed inline, behaviour drift gets a new phase row, decision-needing drift
  gets a `## Blocked` entry, and a bullet the spec *already* flags as open/relocated (per
  `baseline-inventory.md`) is explicitly **not** drift — criterion 4's "no drift" means no
  *unflagged* divergence, since the still-open bullets are this program's declared boundary.
  (b) The four reports are written to `docs/validations/2026-09-04-<slug>-closure.md`, suffixed so
  they do not clobber the earlier *scoped* `2026-09-04-incremental_shapes.md` (partition-grain
  only, commit `7f4358cf`), and the automated-check leg is run once and cited by all four rather
  than four full `cargo test` runs.

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
