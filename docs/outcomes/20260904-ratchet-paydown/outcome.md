# Outcome: Ratchet paydown — reverse the burst's unreviewed hardening creep

**Created:** 2026-09-04
**Status:** queued
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
| 1 | Establish the pre-burst baseline values from git; list every added `unwrap`/`expect`/`println!` site in the burst range | pending |
| 2 | `smelt-db`: classify or convert the added `unwrap` sites | pending |
| 3 | `smelt-cli`: mark legitimate stdout, route the rest through the reporter/`tracing` | pending |
| 4 | Regenerate the baseline with sign-off; record the file-split deferral in `docs/TODO.md`; gates green | pending |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
