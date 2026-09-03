# Phase 3 plan — resolve residual and drifted bullets

## Objective

Close out the two judgement calls phase 2 deliberately left open, so the closure report has a
defensible disposition for every bullet that is not plainly `closed` or `open`. Advances success
criterion 3 (every bullet named in `20260815-definition-delta-migrate` §"Out of scope" is still
accurately worded and still tagged where it should be) and finishes criterion 6's residue question
(phase 2 found zero `residue`, so this phase's job is to confirm nothing else silently reads as
settled, not to reopen an owning outcome).

## Spec delta

None planned up front. This phase *may* produce one: if a `drifted` bullet turns out to be stale
(the underlying behaviour changed but the spec text still describes the old gap), the fix is a
direct edit to the offending `§Known Divergences` / `§Future Extensions` bullet in
`docs/specs/incremental_models.md`, `docs/specs/incremental_shapes.md` or
`docs/specs/model_properties.md` — trivial wording only. **Any stale bullet whose honest fix needs
implementation work, or whose correct wording requires a product decision, does NOT get edited
here**: add a new phase row (implementation) or a `## Blocked` entry (decision) instead, per the
outcome's standing rule.

## Tests

Gates, not unit tests (docs/audit phase):

1. `check-classification.sh` extended — every `drifted` row now also carries a phase-3 `Verdict`
   (`accurate` / `relocated` / `stale-fixed <sha-or-"this commit">`); a `drifted` row with an
   empty or unrecognised verdict fails. Red first: run the extended script before filling the
   column and watch all 16 rows fail.
2. `check-classification.sh` re-run after the column is filled — green, and the existing 80-row
   disposition/presence checks still pass unchanged.
3. `check-inventory.sh` — unchanged, still green (the baseline TSV must not move).

## Tasks

1. Regenerate `current-inventory.tsv` at `HEAD` (`bash extract-baseline.sh HEAD > current-inventory.tsv`)
   so the spot-check reads today's text, not phase 2's snapshot.
2. Add a `Verdict` column to the `baseline-inventory.md` tables (only `drifted` rows populate it;
   others get `-`), and extend `check-classification.sh` to enforce it. Confirm it fails red.
3. Spot-check the 11 "reworded but accurate" `drifted` rows (IM-04, IM-13, IM-14, IM-23, IS-08,
   IS-10, IS-18, IS-22, IS-25, IS-29, MP-06): read each bullet's *current* text and verify against
   the repo that the gap it describes still exists. Record `accurate` or `stale-fixed`.
4. Spot-check the 5 relocated rows (IS-15, IS-24, IS-26, IS-27, IS-28) in `§Future Extensions`:
   verify the relocation is still honest — the item is genuinely undecided-future, not a shipped
   feature left described as future. Record `relocated` or `stale-fixed`.
5. Criterion 3 sweep: for each named item in `docs/outcomes/20260815-definition-delta-migrate`
   §"Out of scope", locate its current spec bullet, and check (a) it still exists, (b) it still
   carries `(Open Question)` where the out-of-scope text claims it does, (c) the behaviour it
   describes as missing is still missing. Record the result in a new
   `## Out-of-scope spot-check` section of `baseline-inventory.md` (one row per named item:
   item, spec + section, tag present?, verdict).
6. Apply any trivial stale-wording spec fixes found in tasks 3–5; for anything non-trivial, add a
   phase row or `## Blocked` entry instead of editing.
7. Cross-check that no file under `docs/outcomes/20260815-keyed-grain-residue` or
   `docs/outcomes/20260815-partition-grain-residue` cites `IS-24` where it means `IS-18`
   (a `rg` sweep; phase 2 believes this is clean — confirm and record the one-liner).
8. Write `phases/03-summary.md`; append a dated Decision-log entry to `outcome.md` and flip row 3
   to `done`.

## Verification

- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-inventory.sh`
- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-classification.sh`
- `bash .claude/scripts/verify-phase.sh`
- If (and only if) a spec bullet was edited: `git diff --stat docs/specs/` reviewed in the summary,
  and note that phase 4's `/smelt:validate` runs are the real oracle for those edits.

## Commit message

`outcome(20260815-incremental-spec-closure-confirm): resolve drifted and out-of-scope bullets`
