# Phase 01 summary — Supersede the 2026-08-16 handoff; confirm the backlog; kill the dangling outcome citation

## Backlog check (test 4)

`.claude/outcome-backlog` already satisfies criterion 2: every queued/active outcome
(`20260904-programme-hygiene` active; `state-residency`, `walk-migration-residue`,
`delta-signature-front-door`, `decided-gap-residue`, `ratchet-paydown` queued) appears above
every `done`/`blocked` outcome (all `20260815-*` and `20260809-*` entries), and the file's header
comment already names `docs/research/20260904-incremental-state-review.md` §"Recommended next
sequence" as the ordering source. No edit needed.

## Handoff banner

Added a blockquote immediately after the H1 and before `**For:**` in
`docs/handoffs/2026-08-16-delta-signature-closure-programme.md`: dated "Superseded (2026-09-04)",
states the programme did not run as written (2026-08-15 queue it retired was in fact executed —
`definition-delta-migrate` and `partition-grain-residue` done, `keyed-grain-residue` blocked;
outcomes 2–6 of §The programme never scaffolded), and points to
`docs/research/20260904-incremental-state-review.md` §"Recommended next sequence" plus
`.claude/outcome-backlog` as the current statement. Added "*Superseded — see the banner.*" markers
under both §The programme and §Immediate actions headings so a reader landing by anchor sees it.
Body left untouched (handoffs are historical).

## `run_state.md` re-point

§Known Divergences bullet "The implementation still keys intervals by calendar-date string"
(line 175) cited `docs/outcomes/20260816-scheduler-delta-signatures/outcome.md`, a directory that
was never scaffolded. Re-pointed to
`docs/research/20260904-incremental-state-review.md` §"Recommended next sequence", which records
that the scheduler delta-signature work is not yet scheduled. Behavioural sentence and the
`20260816-open-questions-triage.md` item 15 decision-record citation unchanged.

## Test results (post-edit)

1. `rg -n '^> \*\*Superseded' docs/handoffs/2026-08-16-delta-signature-closure-programme.md` → line 3, green.
2. `rg -n '20260904-incremental-state-review' .../2026-08-16-*.md` → line 8, green.
3. Dangling-outcome sweep over **tracked** files (`git ls-files docs/specs docs/*.md docs/research
   docs/handoffs .claude | xargs rg -o ...`) → no output, green. Note: the sweep command as
   literally written in the plan (passing directory args directly to `rg`) also picks up
   `.claude/usage-log.jsonl`, a gitignored, ever-growing local log of past shell commands (it
   verbatim-records earlier `rg '20260816-scheduler-delta-signatures'` invocations run during this
   very phase). That file is untracked and not a doc citation — restricting the sweep to
   `git ls-files` output is the correct interpretation and returns clean. It can never go clean
   under the literal directory-arg form since the log keeps recording the phrase forever.
4. Backlog shape — see above, green (no rewrite).
5. `rg -n 'Phase [A-Z0-9]' docs/specs/run_state.md` → no match, exit 1, green (timeless-oracle
   holds; the edited bullet cites a research doc, not a phase label).

## Gates

- `bash .claude/scripts/verify-phase.sh` → `PASS` fmt-check, clippy (both feature sets), workspace
  tests, `example_diagnostics`. `VERIFY: ALL GREEN`.

## For the next planner (phase 2)

- `docs/TODO.md` §"Stale citations flagged by the sweep" does **not** include dangling
  `docs/outcomes/` paths — the only one existing anywhere was the `run_state.md` line this phase
  fixed. Phase 2's sweep should stay scoped to the heading-name citations `docs/TODO.md` actually
  lists; it does not need to repeat the outcome-directory sweep.
- If a future phase wants a clean automated re-run of test 3, exclude `.claude/usage-log.jsonl`
  explicitly (or use `git ls-files`) rather than passing `.claude` as a bare directory arg —
  ripgrep's ignore-file handling is inconsistent across single-file vs. multi-path invocations
  when an untracked file already contains a matching string from earlier tool output.
