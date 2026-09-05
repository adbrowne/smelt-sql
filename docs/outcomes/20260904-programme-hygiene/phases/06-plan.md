# Phase 06 plan — validate the record: `state` + `model_properties` drift, verify-phase green

## Objective

Close success criterion 6 by running `/smelt:validate state` and
`/smelt:validate model_properties` and confirming neither reports drift *introduced by this
outcome* (phases 1–5 edited `state.md`, `run_state.md`, `sources.md`, `schema_evolution.md`,
the timeseries fixtures, and the docs-site incremental-models guide). Also re-verify the
criterion 1–5 assertions still hold as a set, since each was checked only in its own phase.
Pre-existing drift unrelated to this outcome's edits is recorded in `docs/TODO.md`, not fixed.

## Spec delta

None. This phase changes no user-visible behaviour. If validation finds a spec sentence this
outcome *made* wrong (as opposed to one that was already wrong), correct that sentence in the
owning spec — that is closing this outcome's own damage, not new scope.

## Tests

No new tests. This is a docs-only validation phase; the oracle is the two validate runs plus
the standing gates. The per-criterion `rg` assertions below play the role of the red-green list
— each must produce the stated result before the phase is done.

## Tasks

1. Run `/smelt:validate state` (Skill tool, `smelt:validate`, args `state`). Read the drift
   report in full.
2. Run `/smelt:validate model_properties` the same way.
3. For every drift item either run reports, classify it as (a) **introduced by this outcome** —
   traceable to a line phases 1–5 touched (`git log --oneline -6` names the five commits;
   `git show --stat <sha>` gives the file list) — or (b) **pre-existing**.
4. Fix every class-(a) item in the owning spec/doc. Fix nothing in class (b).
5. Append each class-(b) item as one bullet under the existing stale-record section of
   `docs/TODO.md`, naming the spec, the section, and what is wrong in one line.
6. Re-verify criteria 1–5 as a set, foreground, reading each result:
   - `rg -n 'Superseded' docs/handoffs/2026-08-16-delta-signature-closure-programme.md`
     — non-empty, and the banner links `docs/research/20260904-incremental-state-review.md`.
   - `rg -n '20260816-scheduler-delta-signatures' $(git ls-files)` — empty.
   - `rg -n 'Stale citations flagged by the sweep' docs/TODO.md` — empty.
   - `rg -n 'Structure-level degradation behaviours are unevenly specified' docs/specs/state.md`
     — empty.
   - `rg -n 'MP11|F15|ColumnScopedMerge|column.scoped|column_scoped' examples/timeseries/models/daily_events_enriched.sql examples/timeseries/models/daily_events_status.sql examples/timeseries/models/sources/raw/user_status.yml`
     — empty.
   - `rg -n 'ColumnScopedMerge' docs-site/docs/guide/incremental-models.md` — empty.
   Record each PASS/FAIL verbatim in the summary's Gates section.
7. Write `phases/06-summary.md`: the two drift reports condensed, the (a)/(b) split with the
   class-(b) items listed, the six criterion checks, and an explicit statement of whether all
   six success criteria are now met (the next plan step judges the outcome on this).

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be ALL GREEN (this is the doc-sync gate leg
  criterion 6 names; a docs-site or dialect-coverage doc-sync failure here is in scope to fix).
- `cargo test -p smelt-cli --test example_diagnostics` — covered by verify-phase, but name it
  separately in the summary since phases 4–5 edited example workspace models.
- `cargo test -p smelt-cli --test explain_model` — the fixture comments corrected in phases 4–5
  sit next to these assertions.
- The six `rg` checks in task 6.

## Commit message

`docs(programme-hygiene): validate state + model_properties specs carry no outcome-introduced drift`
