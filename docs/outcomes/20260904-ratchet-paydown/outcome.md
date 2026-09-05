# Outcome: Ratchet paydown — reverse the burst's unreviewed hardening creep

**Created:** 2026-09-04
**Status:** active
**Source:** `docs/research/20260904-incremental-state-review.md` §"What went wrong" ("Ratchets crept without sign-off"); `.claude/hardening-baseline.txt`; CLAUDE.md §"Fail-loud discipline"
**Spec anchors:** `docs/specs/architecture.md` §"Fail-loud discipline"

## The outcome

The hardening baseline is back at or below its pre-burst values with each remaining production
`unwrap`/`expect` classified as infallible in a comment, and each `println!` in `smelt-cli`
either legitimate user-facing stdout or converted to the reporter/`tracing`. The `--update`
regeneration is committed with a sign-off note, so the two-sided ratchet is honest again. The
file-split of `execute.rs` and `maintenance_driver.rs` is deliberately left out: it is a
move-only refactor whose safety net exists, but it is judgment-heavy and belongs to a fork-level
implementer, not this loop.

## Success criteria (checkable)

1. `.claude/hardening-baseline.txt` records `smelt-cli println` ≤ 161 and `smelt-db unwrap` ≤ 16
   (the 2026-08-28 pre-burst values, confirmed from git history in phase 1), and no other crate's
   count rises.
2. Every production `unwrap`/`expect` added by commits in the range
   `39228307..994e6f3f` is either converted to `Result`/a diagnostic or carries a one-line
   infallibility justification comment.
3. Every `println!` added to `smelt-cli` in that range is either user-facing stdout (kept, with
   a `// stdout: <reason>` marker) or routed through `RunReporter`/`tracing`.
4. `cargo test -p smelt-core --test hardening_budget` green, with the regenerated baseline
   committed under a message carrying a "ratchet sign-off" line naming this outcome.
5. `verify-phase.sh`, `execute_parity`, `statement_parity` and `maintenance_conformance` green
   (behaviour unchanged).

## Out of scope

- Splitting `crates/smelt-runtime/src/execute.rs` or `maintenance_driver.rs` (each > 6,000
  lines). Recorded in `docs/TODO.md` as a fork-level refactor with the parity gates as its
  safety net.
- Lowering any other ratchet (parser gaps, dialect gaps, registry migration).
- Changing what any diagnostic says.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Establish the pre-burst baseline values from git; census every added `unwrap`/`expect`/`println!` site in `39228307..HEAD`, with a per-site verdict | done |
| 2 | `smelt-db`: classify or convert the added `unwrap` sites | pending |
| 3 | `smelt-cli`: mark legitimate stdout, route the rest through the reporter/`tracing`, and classify or convert the added `expect` sites | pending |
| 4 | Regenerate the baseline with sign-off; record the file-split deferral in `docs/TODO.md`; gates green | pending |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

- 2026-09-06 (plan 01): census range widened from `39228307..994e6f3f` to `39228307..HEAD` — two of
  the added `smelt-cli` printlns landed after `994e6f3f`, and criterion 1 constrains the whole-tree
  count, so the burst-range-only reading would leave it unachievable by construction.
- 2026-09-06 (plan 01): phase 3 extended to own the added `smelt-cli` `expect` sites
  (`commands/migrate.rs` +1, `commands/rebuild.rs` +2). Criterion 2 covers every added
  `unwrap`/`expect`, but phase 2 was scoped to `smelt-db` and phase 3 to `println!` only, so these
  three sites had no phase row.
- 2026-09-06 (plan 01): criterion 1's `smelt-cli println ≤ 161` is at risk — the +10 in
  `commands/migrate.rs` is a new command's user-facing plan output, which criterion 3 explicitly
  keeps. Phase 1 must return a Criterion-1 verdict; if ≤ 161 is unachievable by honest means, the
  phase-4 plan step restates the criterion rather than deleting user-visible output.

- 2026-09-06 (phase 01): census complete (`phases/01-census.md`). All 16 added sites are
  legitimate (`justify`/`stdout`); phase 3's scope shrinks — `commands/rebuild.rs`'s 2 `expect`s
  are a verbatim rename of pre-burst `backbuild.rs` lines, not new sites. Criterion 1 is not
  achievable as literally worded (`smelt-cli println ≤ 161`) — recommended restatement in
  `01-census.md` §3: `smelt-cli println` = 174 with all 13 added sites marked `// stdout:
  <reason>`, `smelt-db unwrap` ≤ 16 (achievable — all 3 additions are pre-justified
  duplicate-pattern sites).

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
