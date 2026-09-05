# Phase 04 plan — sign off the baseline, restate criterion 1, record the file-split deferral

## Objective

Close the outcome: confirm the committed `.claude/hardening-baseline.txt` is the honest
post-paydown state (no crate above its `39228307` value except the two `smelt-cli` rows the
census cleared), apply the census's criterion-1 restatement to `outcome.md`, and record the
`execute.rs` / `maintenance_driver.rs` file-split in `docs/TODO.md` as an explicit fork-level
deferral. Advances criteria 1, 4 and 5 (and discharges the "Out of scope" bullet's promise that
the deferral is *recorded*, not merely omitted).

## Spec delta

None. No user-visible feature behaviour changes; the invariant text in
`docs/specs/architecture.md` §"Fail-loud discipline" and CLAUDE.md already describes the
two-sided ratchet correctly and needs no edit.

## Tests

No new tests. Phase 3 already landed the call-site gate
(`crates/smelt-cli/tests/stdout_markers.rs`) that machine-checks criterion 3, and phase 2 landed
the poison-recovery tests. This phase changes only `outcome.md`, `docs/TODO.md` and (if it drifts)
the baseline file; its red-green oracle is the existing
`hardening_budget::gate_detects_regression` two-sided ratchet, which goes red on any stale row in
either direction.

## Tasks

1. Re-run `bash .claude/scripts/hardening-budget.sh` and confirm "OK". Only if it reports drift,
   run `--update` and inspect the diff line-by-line — an unexpected row moving is a finding, not a
   rubber-stamp.
2. Derive the criterion-1 evidence table with
   `git diff 39228307 HEAD -- .claude/hardening-baseline.txt | grep -E '^[+-][a-z]'` and confirm
   exactly three rows moved: `smelt-cli expect 41→42`, `smelt-cli println 161→174`,
   `smelt-db unwrap 16→6`. Any fourth row is a finding — stop and record it rather than
   regenerating over it.
3. In `outcome.md`, replace criterion 1 with the census `01-census.md` §3 restatement verbatim,
   with the `smelt-db` clause updated to reflect the *achieved* value (6, below the pre-burst 16 —
   phase 2 went further than the census predicted, so the restatement's "≤ 16" is satisfied with
   margin; say so in one clause rather than rewriting the number).
4. Append a dated Decision-log line to `outcome.md` recording the restatement and its rationale
   (user-facing CLI surface of a new command is not ratchet debt).
5. Add a `docs/TODO.md` section — `## Deferred: split `execute.rs` / `maintenance_driver.rs`
   (flagged 2026-09-06, ratchet-paydown phase 4)` — naming both files, their approximate line
   counts, why the loop declines them (move-only but judgment-heavy; belongs to a fork-level
   implementer), and the safety net that makes the move tractable when someone takes it
   (`execute_parity`, `statement_parity`, `maintenance_conformance`).
6. Write `phases/04-summary.md` including the criterion-by-criterion verdict for all five success
   criteria, since this is the outcome's last phase and the completion judgement reads it.
7. Flip phase 4's row to `done` in `outcome.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be ALL GREEN.
- `cargo test -p smelt-core --test hardening_budget --quiet` — the two-sided ratchet.
- `cargo test -p smelt-cli --test stdout_markers --quiet` — criterion 3 still machine-checked.
- `cargo test -p smelt-runtime --test execute_parity --quiet` and
  `cargo test -p smelt-runtime --test statement_parity --quiet` — criterion 5's behaviour-unchanged
  half.
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — criterion 5's equivalence
  half. If it needs a warehouse/env this repo's `verify-phase.sh` does not provide, record the
  skip explicitly in the summary rather than claiming it green.
- `git diff --stat` must touch only `docs/` (and `.claude/hardening-baseline.txt` only if task 1
  found genuine drift).

## Commit message

`docs(outcome): sign off the hardening baseline and record the file-split deferral — ratchet sign-off: 20260904-ratchet-paydown phase 4`
