# Phase 01 — Supersede the 2026-08-16 handoff; confirm the backlog; kill the dangling outcome citation

**Outcome:** `docs/outcomes/20260904-programme-hygiene`
**Advances:** success criteria 1 and 2.

## Objective

A fresh session that opens `docs/handoffs/2026-08-16-delta-signature-closure-programme.md` — the
first thing it reads — must learn within the first screen that the programme it describes was
overtaken by events, and be pointed at `docs/research/20260904-incremental-state-review.md`. The
handoff's queue must also stop being restated as current anywhere else, which today means one
spec line citing an outcome directory that was never scaffolded. Docs-only; no crate changes.

## Spec delta

`docs/specs/run_state.md` §Known Divergences, the bullet "**The implementation still keys
intervals by calendar-date string.**" (line ~175) cites
`docs/outcomes/20260816-scheduler-delta-signatures/outcome.md`, which does not exist and never
did. Re-point it to `docs/research/20260904-incremental-state-review.md` §"Recommended next
sequence" (which records that the scheduler delta-signature work is deliberately not yet
scheduled). Keep the behavioural sentence and the `20260816-open-questions-triage.md` item 15
decision record unchanged — only the forward pointer moves. No user-visible behaviour changes,
so no docs-site edit follows.

## Tests

Docs-only phase; the gates are executable `rg` assertions. Run each before the edit (red), then
after (green).

1. `rg -n '^> \*\*Superseded' docs/handoffs/2026-08-16-delta-signature-closure-programme.md` —
   returns a banner line; red before the edit.
2. `rg -n '20260904-incremental-state-review' docs/handoffs/2026-08-16-delta-signature-closure-programme.md` —
   the banner links the review; red before the edit.
3. Dangling-outcome sweep (red on `run_state.md` before the edit, green after):
   `rg -o --no-filename 'docs/outcomes/[0-9]{8}-[a-z0-9-]+' docs/specs docs/*.md docs/research docs/handoffs .claude | sort -u`
   — every path printed must be an existing directory.
4. Backlog shape: no `done`/`blocked` outcome appears above any outcome whose `outcome.md`
   `**Status:**` is neither, and a comment line names
   `docs/research/20260904-incremental-state-review.md` as the ordering source. Expected green
   already — assert it, do not rewrite the file if it passes.

## Tasks

1. Run test 4's check over `.claude/outcome-backlog`; record the result. Only if it fails, fix
   the order/comment — otherwise leave the file untouched.
2. Add a banner to the handoff, immediately after the H1 and before `**For:**`, as a blockquote:
   `> **Superseded (2026-09-04).**` + two or three sentences saying the programme was replanned
   in flight, that its §The programme sequence did *not* run as written (the 2026-08-15 queue it
   retired was in fact executed — `20260815-definition-delta-migrate` and
   `20260815-partition-grain-residue` are `done`, `20260815-keyed-grain-residue` is `blocked`),
   and that the current statement of the programme is
   `docs/research/20260904-incremental-state-review.md` §"Recommended next sequence" plus
   `.claude/outcome-backlog`. Read the handoff as a historical record from here on.
3. Add one line to the handoff's §The programme and one to §Immediate actions, each saying
   "Superseded — see the banner", so a reader landing mid-document by anchor is not misled.
   Do not delete or rewrite the body (handoffs are historical, like plans).
4. Apply the §Spec delta edit to `docs/specs/run_state.md`.
5. Re-run tests 1–4; all green.
6. Write `phases/01-summary.md`: what the backlog check found, what the banner says, the
   `run_state.md` re-point, and — for phase 2 — that `docs/TODO.md`'s stale-citation list does
   **not** include dangling `docs/outcomes/` paths, so phase 2's sweep should stay scoped to the
   heading names that list names.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- The four `rg` assertions above, run and their output pasted into the summary.
- `rg -n 'Phase [A-Z0-9]' docs/specs/run_state.md` — the timeless-oracle rule still holds for the
  edited bullet (a research-doc link is allowed in Known Divergences; a phase label is not).

## Commit message

`docs(programme-hygiene): supersede the 2026-08-16 handoff and drop the dangling outcome citation`
