# Phase 3b — Blocked before its live-tier tests

**Status: blocked.** No code survives this attempt — the tree was reverted to a clean HEAD.

## What happened

Implemented `partition_literal`'s typed-ANSI rewrite exactly as the 2026-09-14 ruling and the
phase plan specify: `DATE '…'`/`TIMESTAMP '…'` on the calendar axis, parsed via `chrono`, `Err` for
a non-date/timestamp-shaped value, plus the two `unwrap_or_else` fallback sites in
`transformer.rs` and the doc comments on `Region`/`smelt-backend::types.rs`. All six of the plan's
red-green tests were written and passed in isolation (`smelt-logical --test emit_statements`,
`smelt-runtime --lib`).

Task 5 ("run `cargo test --workspace` and repair every fixture pinning the old spelling") surfaced
two categories of failure:

1. **Pinned-text fixtures** — dozens of `assert!`s across `smelt-runtime/src/transformer.rs`,
   `smelt-runtime/src/lib.rs`, `smelt-runtime/tests/compile_parity.rs`,
   `smelt-runtime/tests/partition_axis_windowing.rs` expecting the old bare-quoted spelling. All
   repaired successfully — this part of the plan's Task 5 worked exactly as scoped.
2. **A live DuckDB execution failure** — not a text-pin, a real `Binder Error: Cannot compare
   values of type VARCHAR and type DATE`. See the decision log entry below for the full trace: the
   flagship `examples/web_analytics` raw-events source declares its calendar `partition_column` as
   `VARCHAR` on purpose (a Hive-partitioned-writer shape), and a typed `DATE` literal cannot compare
   against it without an explicit cast. This is category 2's blocker, and it is not a fixture-repair
   problem — it is the 2026-09-14 ruling's premise failing on a real, declared, currently-supported
   shape.

Also discovered along the way (fixed as part of the same attempt, would need to be re-fixed
whichever way 3b eventually lands): the "unreachable" `debug_assert`-guarded fallback in
`inject_source_filters` is NOT unreachable — `smelt_runtime::diagnostics::preview`'s
`placeholder_range()` legitimately passes the symbolic `{{window_start}}`/`{{window_end}}` tokens
through this path for no-`--period` technique previews (`smelt explain --show-sql` with no period,
the LSP's property-diff derivation, etc.). The old lenient `partition_literal` silently accepted
any string on the calendar axis, so this was never exercised; the new strict version panics on it
in debug/test builds (`property_diff_coalescing.rs`'s `a_burst_of_git_change_events_…` test hung
building a panicking task, not on genuine debounce logic — a red herring worth ~40 minutes before
the panic surfaced via `--nocapture`). The fix (a `render_time_literal` wrapper that special-cases
the two known placeholder tokens before delegating to `partition_literal`) is straightforward and
should be folded into whichever attempt eventually lands the calendar-literal fix — it is not itself
blocked on the VARCHAR-column question.

## Decisions

- **2026-09-15 — do not force a resolution to the VARCHAR-vs-typed-literal conflict; block and
  escalate.** See the outcome.md "## Blocked" entry for the full reasoning and the three candidate
  options (thread column type to the renderer; cast the column instead of typing the literal;
  narrow the fix to Trino only). Reverted all code rather than landing a partial/unsound fix,
  per RECORD-AND-CONTINUE: this is a design decision the plan does not answer, not a bug in this
  phase's own target that should proceed anyway.

## For the next planner

- The VARCHAR-partition-column question must be ruled on before 3b (or any successor) can proceed
  — see outcome.md's Blocked entry for the three options and their tradeoffs.
- The `render_time_literal`/symbolic-placeholder fix (described above) is independent of that
  ruling and can be folded into the eventual landing without further design work — it's a pure
  bugfix once "how do we render a real calendar value" is settled.
- `examples/web_analytics/models/sources/raw/events.yml`'s `event_date: VARCHAR` comment
  ("DuckDB casts to DATE on use") suggests DuckDB's *own* internal reads of this column already
  rely on implicit/explicit casting elsewhere in the pipeline — worth checking whether the eventual
  fix could reuse whatever cast convention already exists downstream, rather than inventing a new
  one.
- Not investigated: whether Spark or BigQuery have the same two-directional bug (a VARCHAR-typed
  calendar column colliding with a typed literal) — only Trino (gap 1's original symptom) and
  DuckDB (this attempt's symptom) have been observed. Worth checking as part of whichever option is
  chosen.

## Gates

- Not run to completion — the phase blocked during red-green TDD / fixture repair, before
  `verify-phase.sh`. Ad hoc gates run during the attempt (all against the now-reverted code):
  - `cargo test -p smelt-logical --test emit_statements` — 67 passed (new tests included).
  - `cargo test -p smelt-runtime --lib` — 257 passed after fixture repair.
  - `cargo test -p smelt-runtime --tests --no-fail-fast` — 2 failures found and fixed
    (`source_pushdown_unit`, `statement_parity/staged_candidate_conditional` — the latter is the
    VARCHAR/DATE failure that blocked the phase).
  - `cargo test -p smelt-lsp --test property_diff_coalescing` — initially hung (see above), fixed
    by the `render_time_literal` placeholder special-case, then passed.
  - Final state: `git status` clean, all edits reverted.
