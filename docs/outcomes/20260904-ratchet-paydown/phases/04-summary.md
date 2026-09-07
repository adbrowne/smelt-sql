# Phase 04 summary — sign off the baseline, restate criterion 1, record the file-split deferral

## Shipped

- `outcome.md`: criterion 1 replaced verbatim with the census's restatement (`smelt-db unwrap` ≤
  16, achieved at 6; `smelt-cli println` = 174 with all 13 added sites call-site-marked).
- `outcome.md`: top-level Status flipped `active` → `done`; phase 4 row flipped `planned` → `done`;
  Decision log closed out with the phase-4 verdict.
- `docs/TODO.md`: new `## Deferred: split execute.rs / maintenance_driver.rs` section — names both
  files (~6,650 / ~6,160 lines), why the loop declines the split (move-only but judgment-heavy),
  and the safety net (`execute_parity`, `statement_parity`, `maintenance_conformance`) that makes
  it tractable for a fork-level implementer later.

## Decisions

- No baseline regeneration was needed — `hardening-budget.sh` already reports OK, and
  `git diff 39228307 HEAD -- .claude/hardening-baseline.txt` shows exactly the three rows the
  census predicted (`smelt-cli expect 41→42`, `smelt-cli println 161→174`, `smelt-db unwrap
  16→6`), no fourth row. Task 1's stop condition did not trigger.
- Criterion 1's `smelt-cli` bound became an exact `= 174` rather than a ceiling: all 13 added
  sites are legitimate new/expanded user-facing CLI output (`smelt migrate` alone is 10), so a
  `≤ 161` ceiling would force deleting shipped, tested output rather than catching unreviewed
  chatter — the census's honest-means argument, applied verbatim.

## For the next planner

- Outcome closed — all 5 success criteria met (see verdict below). No follow-up phase for this
  outcome.
- The `execute.rs` / `maintenance_driver.rs` file split remains open and is now durably recorded
  in `docs/TODO.md` rather than only in this outcome's "Out of scope" bullet — pick it up as its
  own fork-level task when someone has room to iterate on the module seams.
- Nothing else surfaced during verification; all five gates were clean on first run with no
  investigation needed.

## Criterion-by-criterion verdict

1. **Met (restated).** `smelt-db unwrap` = 6 (≤ 16, with margin); `smelt-cli println` = 174, all
   13 added sites marked; no other crate's count exceeds its `39228307` value.
2. **Met.** All added `unwrap`/`expect` sites across the range are either converted (`smelt-db`
   phase 2) or carry an infallibility/`stdout` justification comment (`smelt-cli` phase 3).
3. **Met.** All 13 added `println!`/`eprintln!` sites in `smelt-cli` carry `// stdout: <reason>`,
   machine-checked by `crates/smelt-cli/tests/stdout_markers.rs`.
4. **Met.** `cargo test -p smelt-core --test hardening_budget` green; baseline was already
   accurate (no regeneration needed this phase; phase 2 committed the last genuine change).
5. **Met.** `verify-phase.sh`, `execute_parity` (4/4), `statement_parity` (37/37), and
   `maintenance_conformance` (81/81) all green.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, workspace
  tests, example_diagnostics).
- `cargo test -p smelt-core --test hardening_budget --quiet` — 4/4 passed.
- `cargo test -p smelt-cli --test stdout_markers --quiet` — 3/3 passed.
- `cargo test -p smelt-runtime --test execute_parity --quiet` — 4/4 passed.
- `cargo test -p smelt-runtime --test statement_parity --quiet` — 37/37 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — 81/81 passed.
- `git diff --stat` — touches only `docs/TODO.md` and `docs/outcomes/20260904-ratchet-paydown/outcome.md`.
