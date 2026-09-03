# Phase 2 summary — classify baseline bullets against repo state

## Shipped

- `extract-baseline.sh` — now takes an optional `[ref]` argument (default `03a431f3`); re-running
  with no argument is still byte-identical to the committed `baseline-inventory.tsv`.
- `current-inventory.tsv` — the extractor run at `HEAD`, the machine artifact phase 2 joined
  against for presence/absence.
- `baseline-inventory.md` — all 80 `Disposition` cells filled in (`closed <sha>` / `open` /
  `drifted (...)`), plus a `## Classification summary` section (counts table, no-`residue`
  finding, and the `drifted` worklist split into "reworded but accurate" vs "moved to §Future
  Extensions").
- `check-classification.sh` — new gate: every row has a valid disposition, every `closed` sha
  resolves (`git cat-file -e`), every `closed`/`residue` lead-in is absent from the current Known
  Divergences text, every `open` lead-in is present verbatim. `drifted` rows are exempt from the
  presence/absence check by design (see script header comment) — the whole point of `drifted` is
  that wording changed, so an exact-match check would misclassify either direction.

## Totals

35 `closed`, 29 `open`, 16 `drifted`, **0 `residue`**. Per spec: `definition_deltas` 7/0/0/0,
`incremental_models` 14/7/4/0, `incremental_shapes` 12/9/11/0, `model_properties` 2/13/1/0.

## Decisions

- Four parallel research subagents (one per spec) did the repo-verification legwork; I compiled
  and cross-checked their findings against `current-inventory.tsv` before writing dispositions —
  every `closed` sha was independently re-verified with `git cat-file -e` (all 29 distinct shas
  resolve).
- `drifted` split into two flavors in the classification summary: reworded-but-still-accurate
  (11 bullets) vs. moved to `## Future Extensions` as a deliberate 2026-08-16 reclassification
  (5 bullets: IS-15, IS-24, IS-26, IS-27, IS-28). Neither flavor is a spec bug — both are "the
  gate can't tell drift-as-decay from drift-as-honest-update," so phase 3's job on this list is a
  spot-check, not an assumed fix.
- **Found and fixed a mislabeling from phase 1's decision log**: the transactional-merge-ledger
  bullet (the one `20260815-keyed-grain-residue` phase 3 is blocked on) is `IS-18` ("The
  reconciliation ledger's fold is transactional on DuckDB only"), not `IS-24` ("Locality open
  questions") as phase 1's decision log and phase 2's own planning entry both said. `IS-24` is a
  distinct bullet about recurrence-bound slice pruning / granularity relaxation / slice-scoped
  deletion. Both are now `drifted` for unrelated reasons (`IS-18`: reworded bold lead-in, same
  DuckDB-only gap; `IS-24`: moved to §Future Extensions).
- **`IS-18` resolved to `drifted`, not `residue`**: `keyed-grain-residue` phase 3 is blocked, but
  its decision log honestly states "Criterion 3 is deliberately left unmet" — it never claims
  closure, so success criterion 6 (residue = false-closure claim) doesn't fire. The bullet's bold
  lead-in did change (an added clause about the ledger table's existence), which is why it's
  `drifted` rather than plain `open` under this gate's exact-match rule for `open`.
- Ended up with **zero `residue` rows** — every closure an owning outcome's decision log claims
  was independently confirmed against code/tests/a landed decision record. This is a real (if
  quiet) finding for the closure report: the 2026-08-15 program's self-reported closures hold up.

## For the next planner

- Phase 3's worklist per the plan: spot-check the 16 `drifted` rows' *current* wording for
  staleness (not just relocation) — `baseline-inventory.md` §Classification summary already notes
  none looked stale on this pass, but phase 3 owns the final call.
- No owning-outcome reopen is needed (zero `residue`), so phase 3's scope is lighter than the
  outcome's phase-1 decision log anticipated — it's the wording spot-check plus the mislabeled-ID
  correction (already fixed here, but phase 3/5 should double check nothing else in
  `docs/outcomes/20260815-keyed-grain-residue` or `docs/outcomes/20260815-partition-grain-residue`
  cites `IS-24` where it means `IS-18`).
- `IM-13`'s "Open Question" tag was dropped along with `IM-17`/`IM-18`/`IM-20`/`IM-21`/`IM-22`'s
  full closure via the `docs/research/20260816-open-questions-triage.md` decision sweep — worth a
  one-line mention in the phase-5 closure report as the mechanism behind most `2e1f6d19` closures.

## Gates

- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-inventory.sh` — OK (80
  bullets, regression guard held through the ref-parameterization change).
- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-classification.sh` — OK (all
  80 dispositions valid and repo-verified).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full test
  suite, example_diagnostics). Docs/script-only change; no Rust source touched.
