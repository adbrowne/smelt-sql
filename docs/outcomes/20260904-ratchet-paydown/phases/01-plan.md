# Phase 01 — Census: pre-burst baseline + every added site

## Objective

Establish, from git rather than from memory, the pre-burst hardening-baseline values and an
exhaustive per-site census of the production `unwrap`/`expect`/`println!` added since. The census
is the input to phases 2–4 and the evidence for success criteria 1–3; it also has to return a
verdict on whether criterion 1's `smelt-cli println ≤ 161` is achievable by honest means, because
some added sites are a new command's user-facing stdout.

## Spec delta

None. This phase changes no user-visible behaviour and no production code — it produces a census
document only.

## Recon already done (start here, verify, do not redo from scratch)

Pre-burst baseline is `39228307` (2026-09-03): `smelt-cli println 161`, `smelt-cli expect 41`,
`smelt-db unwrap 16`. HEAD baseline: `smelt-cli println 174`, `smelt-cli expect 42`,
`smelt-db unwrap 19`. All other crate/pattern rows are unchanged across the range. The deltas
localise to:

- `smelt-db unwrap` +3 — all three in `crates/smelt-db/src/lib.rs`.
- `smelt-cli println` +13 — `commands/migrate.rs` +10, `commands/history.rs` +1,
  `commands/status.rs` +1, `commands/run.rs` +1.
- `smelt-cli expect` net +1 — `commands/migrate.rs` +1, `commands/rebuild.rs` +2,
  `commands/backbuild.rs` −2 (file removed).

Two of the 13 printlns landed *after* `994e6f3f`; the census range is therefore `39228307..HEAD`,
not the range named in the outcome's criteria 2–3.

## Deliverable

`docs/outcomes/20260904-ratchet-paydown/phases/01-census.md`, containing:

1. The pre-burst vs HEAD baseline table for the three changed crate/pattern rows, each value
   quoted from `git show <rev>:.claude/hardening-baseline.txt` with the rev named.
2. One row per added site: `file:line`, the one-line source excerpt, the introducing commit
   (`git log -S` or `git blame`), and a proposed verdict from a fixed vocabulary:
   - `convert` — becomes `Result`/a diagnostic (unwrap/expect),
   - `justify` — genuinely infallible, gets a one-line comment (unwrap/expect),
   - `stdout` — legitimate user-facing command output, kept with a `// stdout: <reason>` marker,
   - `route` — diagnostic/progress chatter, moves to `RunReporter`/`tracing`.
3. A reconciliation line per crate/pattern proving `pre-burst + added − removed == HEAD baseline`.
4. A **Criterion-1 verdict** section: given the `stdout` verdicts, state the lowest honest
   `smelt-cli println` count and whether it is ≤ 161. If it is not, recommend the exact
   restatement of criterion 1 (e.g. "no non-`stdout` println remains, and the count is ≤ 161 plus
   the marked `stdout` lines"), for the phase-4 plan step to apply.

## Tests

No production code changes, so no red-green unit test. The checks that stand in for one, all run
and their output pasted into the summary:

- `bash .claude/scripts/hardening-budget.sh` — must report OK (tree matches baseline) before and
  after this phase; proves the census's HEAD numbers are the gate's numbers, not a hand count.
- Reconciliation arithmetic (item 3 above) must balance exactly for all three rows; a mismatch
  means a site was missed and the census is not done.

## Tasks

1. Re-derive both endpoint baselines with `git show <rev>:.claude/hardening-baseline.txt`; confirm
   the recon table above and correct it if it differs.
2. For each of the three changed crate/pattern rows, diff the per-file production counts between
   `39228307` and HEAD using the same counting rule as `hardening-budget.sh` (skip `tests.rs`,
   `main.rs`, `*/tests/*`, and everything from the first `#[cfg(test)]` line).
3. For each file with a positive delta, locate the exact added lines and attribute each to its
   introducing commit.
4. Assign every site a verdict from the four-word vocabulary; for `justify` state the reason that
   will become the comment, for `route` name the reporter/tracing call it becomes.
5. Write `01-census.md`, including the reconciliation and the Criterion-1 verdict.
6. Write `01-summary.md`: the verdict tallies per phase (how many sites phase 2 owns, how many
   phase 3 owns), the Criterion-1 recommendation, and anything that should reshape phases 2–4.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-core --test hardening_budget --quiet 2>&1 | tail -20`

## Commit message

`outcome(20260904-ratchet-paydown): census the pre-burst baseline and every added hardening site`
