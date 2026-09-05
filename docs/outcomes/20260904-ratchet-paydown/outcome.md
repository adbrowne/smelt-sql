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
| 2 | `smelt-db`: convert the added `unwrap` sites (shared poison-recovering registry accessors) | done |
| 3 | `smelt-cli`: mark the 13 added `println!`/`eprintln!` sites `// stdout: <reason>` and justify `migrate.rs:403`'s `expect` (not `rebuild.rs`) | done |
| 4 | Regenerate the baseline with sign-off; record the file-split deferral in `docs/TODO.md`; gates green | planned |

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

- 2026-09-06 (plan 02): resolved the internal contradiction in `01-census.md` — its §3 restatement
  keeps `smelt-db unwrap ≤ 16` while its "Phase ownership" section says the count "stays 19". §3
  wins: the 13 `RwLock::read()/write().unwrap()` sites in `smelt-db/src/lib.rs` collapse to two
  poison-recovering accessors, taking the crate to ~6 without changing any reachable behaviour, so
  criterion 1's `smelt-db` half is met by honest means rather than restated. Phase 2 is therefore a
  conversion phase, not verification-only, and touches ten pre-existing sites alongside the three
  added ones (same file, same mechanical pattern).
- 2026-09-06 (plan 02): phase 2 runs `hardening-budget.sh --update` for its own count drop (the
  ratchet is two-sided, so leaving it stale would fail `verify-phase.sh`); phase 4 still owns the
  whole-baseline regeneration, the criterion-1 restatement, and the `docs/TODO.md` deferral note.
  Phase 4's restatement now only needs to change the `println` clause.
- 2026-09-06 (plan 02): phase 3's row retitled to match the census correction — `rebuild.rs`'s two
  `expect`s are a verbatim rename of pre-burst `backbuild.rs` lines and are out of scope.

- 2026-09-06 (implement 02): phase 2 done — `smelt-db unwrap` 19 → 6 via shared
  `read_registry`/`write_registry` accessors, baseline regenerated (single-line diff), all gates
  green (`verify-phase.sh`, `hardening_budget`, `execute_parity`). Found and worked around a live
  deadlock in an early draft of the poison-recovery tests: cloning the whole `Database` (not just
  the target `RwLock`) before poisoning stands up a Salsa snapshot, and a subsequent `&mut self`
  mutation on the original handle blocks on Salsa's cancellation machinery. Fixed by cloning only
  the `Arc<RwLock<_>>` field; not a code-path change, so not tracked as a new TODO.

- 2026-09-06 (plan 03): no reshape — phase 3's row already carries the census correction. Added a
  scope note rather than a new row: the phase ships a source-text gate
  (`crates/smelt-cli/tests/stdout_markers.rs`) so criterion 3 is machine-checked at the call sites
  instead of asserted only in the census prose, and a future unmarked stdout site in `migrate.rs`
  fails a test. The `diff.rs:502` comment alignment rides along as free consistency work (that site
  predates the range and no criterion reaches it).

- 2026-09-06 (implement 03): phase 3 done — all 10 added `println!` sites in `migrate.rs`, the 2
  added stateless-mode sites (`history.rs`, `status.rs`), and the second `run.rs` selector-miss
  site now carry `// stdout: <reason>` markers; `migrate.rs`'s added `.expect(...)` carries an
  `// invariant:` justification, mirrored onto the pre-existing `diff.rs:502` twin for
  consistency. New gate `crates/smelt-cli/tests/stdout_markers.rs` machine-checks this at the
  call sites. Zero hardening-budget counts moved; `verify-phase.sh` all green.

- 2026-09-06 (plan 04): no reshape — phase 4 is the outcome's last row and the census (phase 1 §3)
  already fixed its scope. Scope note: the baseline was regenerated by phase 2 (the `smelt-db`
  drop) and is currently OK, so phase 4's "regenerate" task is a *verification with a stop
  condition* — exactly three rows may differ from `39228307`; a fourth is a finding, not something
  to regenerate over. Phase 4 also owns the criterion-1 restatement, the `docs/TODO.md` file-split
  deferral, and a criterion-by-criterion verdict in its summary (the completion judgement reads
  it).

## Blocked

<!-- Dated entries; each names the phase, what blocked it, and what a human must decide. -->
