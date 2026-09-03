# Phase 2 plan — classify every baseline bullet against current repo state

## Objective

Fill in the `Disposition` column for all 80 baseline bullets in `baseline-inventory.md` by joining
the baseline inventory against the *current* (HEAD) spec text and the three owning outcomes'
decision logs, classifying each as `closed` / `open` / `drifted` / `residue`. This is the
evidence base for success criteria 1, 2, 3 and 6; phase 3 acts on whatever `drifted` and
`residue` rows this phase names.

## Spec delta

None. This phase reads specs and writes outcome-local artifacts only; it changes no user-visible
behaviour. Any spec edit implied by a `drifted` row is phase 3's work, not this phase's.

## Classification vocabulary (fixed here, used by the gate)

- `closed <commit>` — bullet absent from the current §Known Divergences section **and** the repo
  independently shows the behaviour landed; cite the commit that removed it.
- `open` — bullet still present, still accurately worded, still needs a product decision this
  program declined to make.
- `drifted` — bullet still present in the spec but its underlying behaviour changed (the entry is
  stale), or absent from the spec but not actually implemented (removed prematurely). Both
  directions are drift bugs for phase 3.
- `residue` — an owning outcome claims the bullet closed, but the repo does not independently
  confirm it (criterion 6). `IS-24` is the known candidate.

## Tests (red-green)

1. `extract-baseline.sh` grows an optional ref argument (`extract-baseline.sh [ref]`, default
   `03a431f3`); re-running it with no argument must produce a byte-identical `baseline-inventory.tsv`
   — assert via `check-inventory.sh`, which must stay green throughout (regression guard).
2. `check-classification.sh` (new gate, must be verified red before the dispositions are written):
   - every one of the 80 baseline IDs has a `Disposition` cell that is **not** `TBD` and whose
     first word is one of `closed|open|drifted|residue`;
   - every `closed` row cites a commit that `git cat-file -e` resolves;
   - every `closed`/`residue` row's bold lead-in is **absent** from the current spec's §Known
     Divergences section, and every `open`/`drifted`-still-present row's lead-in is **present**
     (comparison via the same whitespace-collapsing extractor run at `HEAD`);
   - exits non-zero listing offending IDs.
3. `check-inventory.sh` — still green (baseline table untouched apart from the Disposition column;
   extend it only if its row-count/lead-in assertions break on the new column).

## Tasks

1. Parameterize `extract-baseline.sh` by git ref; confirm `check-inventory.sh` still passes.
2. Generate `current-inventory.tsv` (extractor at `HEAD`) as the machine artifact for the join;
   commit it alongside the baseline TSV.
3. Write `check-classification.sh` per test 2; run it and confirm it fails loudly (all 80 `TBD`).
4. Mechanically join baseline ↔ current by (spec, collapsed bold lead-in); produce the
   present/absent split. Note lead-ins that were *reworded* rather than removed — those join by
   hand and are candidates for `open` (reworded, still live) not `closed`.
5. For each **absent** bullet: find the removing commit (`git log -S'<lead-in fragment>' --
   docs/specs/<spec>.md`), then verify the claim against the repo rather than the commit message
   (the named test/gate exists and passes, or the named code path exists). Classify `closed
   <sha>` or `residue`.
6. For each **present** bullet: read the current wording and spot-check the behaviour it describes
   is still absent. Classify `open` or `drifted`.
7. Special-case `IS-24` (transactional ledger fold): `20260815-keyed-grain-residue` is `blocked` on
   exactly this. Verify the fold's backend coverage in the repo directly; classify `residue`
   unless the repo independently shows every shipped backend folds transactionally.
8. Cross-read the three owning outcomes' decision logs (`definition-delta-migrate`,
   `keyed-grain-residue`, `partition-grain-residue`) for any bullet they *claim* to close that
   step 5 did not confirm; every such mismatch becomes `residue`.
9. Write the dispositions into `baseline-inventory.md` (one cell per row) and add a short
   `## Classification summary` section: counts per class, and the ID list for `drifted` +
   `residue` (phase 3's worklist).
10. Append the phase-2 findings to `phases/02-summary.md` (counts, the drifted/residue worklist,
    anything phase 3 must decide).

## Verification

- `bash .claude/scripts/verify-phase.sh` (docs/script-only change; must stay green).
- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-inventory.sh`
- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-classification.sh`

## Commit message

`outcome(20260815-incremental-spec-closure-confirm): classify baseline bullets against repo state`
