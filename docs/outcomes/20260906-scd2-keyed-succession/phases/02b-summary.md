# Phase 2b summary — pay down the `hardening_budget` regression

## Shipped

- `crates/smelt-logical/src/analysis/succession.rs`: the single-window select-list arm
  (`match windows.len() { 1 => ...unwrap()... }`) is now destructured via `windows.pop()`
  matched on `Some`/`None`, eliminating the `unwrap()`.
- New `WindowShape` struct + `window_shape(alias, window_call) -> Result<WindowShape,
  NotSuccessionReason>` helper: extracts one window call's own per-item checks (LEAD/LAG name,
  single bare-column argument, `PARTITION BY` key set, single ascending `ORDER BY` column) with
  no `Option`-accumulation and no `expect()`.
- The former `Option`-accumulating shared-window loop is replaced by `window_items
  .split_first()`: the `None` arm is the existing "no LEAD/LAG window projection found" refusal,
  and the `Some` arm seeds `clock_col` / `partition_cols` / `shared_order_text` /
  `shared_order_expr` as plain non-`Option` values from the first item, then folds the rest
  against them. New `record_window(...)` helper shares the "does the call reach over the clock
  column, then record lead/lag" check between the first item and the fold loop.
- Two new unit tests pinning previously-uncovered refusal paths:
  `refuses_order_by_expression_not_bare_column` and `refuses_two_window_calls_in_one_projection`
  (the latter needed `COALESCE(LEAD(...), LAG(...))`, not a bare `=` comparison, to get the
  parser to find two window calls in one projection — a bare binary comparison at the top of a
  select-list item didn't reproduce it; recorded here so a future author doesn't rediscover it).

## Decisions

- Eliminated all four `unwrap`/`expect` sites via restructuring rather than classifying them as
  infallible — `hardening-budget.sh` is a pure count ratchet with no allowlist, so elimination
  was the only baseline-honouring move, per the plan.
- Kept every existing refusal's reason variant, wording, and ordering byte-identical; the 39
  pre-existing succession tests pass unedited, confirming the restructure is behaviour-preserving.

## For the next planner

- **New finding, caused by this phase's own diff**: `verify-phase.sh`'s workspace `cargo test`
  leg now fails on `smelt-core`'s `large_file_ratchet::gate_passes_on_committed_tree` —
  `crates/smelt-logical/src/analysis/succession.rs` grew from the committed baseline of 1229
  lines to 1282 (the `WindowShape`/`window_shape`/`record_window` extraction plus two new tests,
  net of the duplication removed). Trimmed doc comments and merged the "reaches over clock
  column" check into a shared `record_window` helper to hold the growth down, but could not get
  back under baseline without either dropping the plan-directed extraction or splitting the file
  into submodules — the latter is out of scope for a hardening-ratchet fix. Per
  `docs/outcome_loop.md` §"The large-file shrink step", this ratchet is designed to be paid down
  by a separate, non-blocking automated step the outcome loop dispatches between iterations
  (`.claude/outcome-shrink-prompt.txt`), not by the phase that triggered it — flagging here in
  case that step doesn't fire before the next planner reads this. Not fixed in this phase: it is
  a different gate than the one 2b targeted, and the plan's own Verification section anticipated
  this residual ("If one does [fail], it is a new finding for the next planner, recorded in the
  summary rather than papered over").
- All other `verify-phase.sh` legs are green: fmt, clippy (both feature sets), the full
  `cargo test` output otherwise, and `example_diagnostics`.

## Gates

- `bash .claude/scripts/hardening-budget.sh` — exit 0, `smelt-logical unwrap 1` / `expect 1`,
  `.claude/hardening-baseline.txt` untouched (`git diff --stat` empty).
- `cargo test -p smelt-core --test hardening_budget --quiet` — 4 passed.
- `cargo test -p smelt-logical --lib` — 914 passed (39 succession tests, including the 2 new
  ones).
- `cargo test -p smelt-logical --test walk_coverage` — 8 passed.
- `cargo fmt --all -- --check` — clean.
- `bash .claude/scripts/clippy-gate.sh` — zero warnings, both feature sets.
- `bash .claude/scripts/verify-phase.sh` — **not fully green**: fails only on
  `large_file_ratchet::gate_passes_on_committed_tree` (see "For the next planner" above); every
  other leg passes.
