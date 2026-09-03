# Phase 5 plan — write `closure-report.md`; final standing-gate run

## Objective

Produce the outcome's one checkable deliverable, `closure-report.md`, which enumerates all 80
baseline bullets with a disposition (success criterion 1), records the criterion-2/3/6 evidence
phases 2–3 established, cites the criterion-4 validation reports, and carries the criterion-5
standing-gate results from a run made *in this phase*. No spec or code behaviour changes.

## Spec delta

None. This phase produces an outcome-local audit artifact only; phases 1–4 already made every spec
edit this program needed (phase 4 was the last, and produced two doc/wording fixes).

## Tests

Red-green via a new gate script, `check-closure-report.sh` (same pattern as the three existing
check scripts in this directory). Confirm it is **red before** the report exists and green after:

- `report_exists` — `closure-report.md` is present in the outcome directory.
- `every_baseline_id_enumerated` — every ID in `baseline-inventory.tsv`'s derived ID space
  (`DD-01..07`, `IM-01..25`, `IS-01..32`, `MP-01..16`; 80 total) appears in the report's
  disposition table exactly once.
- `closed_ids_cite_a_sha` — every row the report dispositions `closed` names a 8+-hex commit sha.
- `open_ids_state_a_reason` — every row dispositioned `open` carries a non-empty reason cell (this
  is the part criterion 1 demands that `baseline-inventory.md` does not yet record).
- `all_six_criteria_sectioned` — the report has one section per success criterion 1–6.
- `criterion_5_gates_all_named` — all five criterion-5 gate commands appear in the report with a
  PASS/FAIL verdict beside each.

## Tasks

1. Write `check-closure-report.sh` (executable, `set -euo pipefail`, derives the 80 expected IDs
   from `baseline-inventory.tsv` rather than hard-coding them); run it, confirm red.
2. Run the five criterion-5 gates in the foreground, capturing each verdict:
   `bash .claude/scripts/verify-phase.sh`, `cargo test -p smelt-cli --test maintenance_conformance`,
   `cargo test -p smelt-runtime --test statement_parity`,
   `cargo test -p smelt-logical --test walk_coverage`,
   `cargo test -p smelt-runtime --test execute_parity` (each `2>&1 | tail -40`).
3. Draft `closure-report.md` §Criterion 1: the 80-row disposition table, one row per ID
   (`ID | spec | bullet lead-in | disposition | evidence`), joined from `baseline-inventory.md`.
   For the 29 `open` rows, write the missing per-bullet reason — the product decision this program
   declined to make (cite the outcome's §Out of scope boundary, or the owning spec's
   `§Future Extensions` framing, per row). Do not re-derive dispositions; phase 2/3 own them.
4. §Criterion 2: name each bullet `keyed-grain-residue` / `partition-grain-residue` claim to close
   and show it is absent from the current `incremental_shapes.md` §Known Divergences (grep-backed,
   citing `current-inventory.tsv`).
5. §Criterion 3: cite `baseline-inventory.md` §"Out-of-scope spot-check" and carry forward the one
   footnote finding (the `20260704-model-updates.md` D3 leg staleness in the *other* outcome's
   historical prose, not in any anchor spec).
6. §Criterion 4: cite the four `docs/validations/2026-09-04-<slug>-closure.md` reports and their
   0/0/1/1 drift counts, all dispositioned `fixed this phase` in phase 4.
7. §Criterion 5: the table of five gates with task-2 verdicts.
8. §Criterion 6: state that phase 2 found **zero** `residue` bullets, and record the
   `20260815-keyed-grain-residue` blocked-phase-3 (`IS-18`) situation explicitly: that outcome
   never claimed closure, so criterion 6 does not fire and no owning outcome is reopened.
9. Re-run `check-closure-report.sh` (green) plus the three existing check scripts
   (`check-inventory.sh`, `check-classification.sh`, `check-validations.sh`).
10. Judge the six criteria: if all met, append a dated evidence line to the outcome's Decision log,
    set `**Status:** done`, and flip row 5 to `done`. If any is unmet, flip row 5 to `blocked` with
    a dated `## Blocked` entry instead — do not paper over it.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- The four remaining criterion-5 gates (task 2)
- `bash docs/outcomes/20260815-incremental-spec-closure-confirm/check-closure-report.sh`
- `check-inventory.sh`, `check-classification.sh`, `check-validations.sh` (all still green)

## Commit message

`outcome(20260815-incremental-spec-closure-confirm): write closure report and run standing gates`
