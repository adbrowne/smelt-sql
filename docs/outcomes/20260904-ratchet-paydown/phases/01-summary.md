# Phase 01 summary — Census

**Shipped:** `docs/outcomes/20260904-ratchet-paydown/phases/01-census.md` — the full baseline
table, per-site verdict table for all 16 added sites (3 `smelt-db` unwrap, 13 `smelt-cli`
println, 1 `smelt-cli` expect), reconciliation arithmetic for all three rows, and a Criterion-1
verdict. No production code changed.

**Decisions:**
- Every added `smelt-db` unwrap (3, all `a68a8268`) is already `justify`-verdict: the introducing
  commit copied the file's existing RwLock-poisoning-rationale comment convention verbatim. Phase
  2 becomes verification-only, not conversion work.
- Every added `smelt-cli` println (13) is `stdout`-verdict — 10 are the entire rendering surface
  of the new `smelt migrate` command, 3 are new branches/call-sites of pre-existing user-facing
  messages. None is diagnostic chatter; there is nothing for phase 3 to route through
  `tracing`/`RunReporter`.
- **Correction to the outcome's own recon**: `commands/rebuild.rs`'s 2 `expect`s are *not* added
  sites — `5ea4c9fb` renamed `backbuild.rs` → `rebuild.rs` verbatim, and `backbuild.rs` already
  carried both lines at the pre-burst commit `39228307`. Phase 3's real scope is the single
  `migrate.rs:403` expect (`justify`, mirrors the pre-existing `diff.rs:502` pattern).
- Criterion 1 (`smelt-cli println ≤ 161`) is **not achievable by honest means** — recommended
  restatement: `smelt-cli println` = 174 with all 13 added sites individually marked
  `// stdout: <reason>`, `smelt-db unwrap` reversed to ≤ 16 pre-burst value (all 3 additions are
  pre-justified duplicate-pattern sites, so a straight revert-to-baseline reading is achievable
  there — see phase 2). Recorded verbatim in `01-census.md` §3 for the phase-4 plan step to apply
  to `outcome.md`.

**For the next planner:**
- Phase 2 is now confirm-only: check the 3 comments at `smelt-db/src/lib.rs:696/713/730` (via
  their leading `// invariant: same RwLock poisoning rationale as set_source_file.` lines) are
  present and accurate — they are, per this census, but the plan should still make it a checked
  step rather than skip it outright.
- Phase 3 should drop `rebuild.rs` from its task list entirely (see correction above) and instead
  cover: 13 `// stdout:` markers across `migrate.rs` (10), `history.rs` (1), `status.rs` (1),
  `run.rs` (1), plus one justification comment on `migrate.rs:403`. Optionally (not required by
  any criterion) mirror that comment onto the pre-existing `diff.rs:502` twin for consistency —
  flagged as a nice-to-have, not scheduled.
- Phase 4 needs the criterion-1 restatement text from `01-census.md` §3 applied to `outcome.md`
  before regenerating the baseline, otherwise the regenerated baseline will look like it's failing
  a criterion that was never achievable as originally worded.

**Gates:**
- `bash .claude/scripts/hardening-budget.sh` — OK, before and after (no production code touched).
- `cargo test -p smelt-core --test hardening_budget --quiet` — 4 passed (includes the
  intentional-regression detector test, which asserts the gate still flags a synthetic
  regression; unrelated to this outcome).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace test suite, example_diagnostics).
