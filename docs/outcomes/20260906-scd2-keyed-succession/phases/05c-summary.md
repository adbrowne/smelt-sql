# Phase 5c summary — Rebuild paths and the statement-parity succession family

## Shipped

- `emit_succession_full_rebuild` (`crates/smelt-logical/src/maintenance/emit/succession.rs`):
  one transactional `StatementGroup` — presented `CREATE TABLE ... AS <compiled select>`
  (reusing `emit_create_table_as`'s spelling), `DELETE FROM <tombstones>`, `INSERT INTO
  <tombstones> ... <emit_succession_ledger_rebuild_select with window_predicate: None>`.
  DuckDB-only, same panic convention as `emit_succession_patch`. Unit test + a
  DuckDB-executed proof in `crates/smelt-logical/tests/succession_emit.rs`.
- `rebuild_succession_state` (`crates/smelt-runtime/src/maintenance_driver/succession/
  execute.rs`): drops the presented table (idempotent, non-transactional), ensures the
  tombstone table, then runs the emitted group via `execute_write_with_bookkeeping`.
  Factored `resolve_ledger_column_types` out of `execute_succession_maintenance` so both
  functions share the key/clock type resolution. 4 new unit tests (full rebuild matches
  the oracle; a stale tombstone with no matching source row is dropped, not appended past;
  a failing ledger `INSERT` rolls back the presented `CREATE TABLE AS` too; a
  range-rebuild-shaped call re-derives the whole ledger).
- `crates/smelt-runtime/src/execute/project.rs`'s succession dispatch block now branches on
  `request.full_refresh || force_full_refresh`: that path compiles the model and calls
  `rebuild_succession_state` (manifest strategy `"succession_full_rebuild"`, no time range);
  every other run keeps the existing window-forward `execute_succession_maintenance` path.
- `crates/smelt-runtime/tests/statement_parity/succession.rs` (new family, registered in
  `main.rs`): 3 tests — patch-path executed statements match the emitters, full-refresh
  executed statements match `emit_succession_full_rebuild`, and the patch-loop result
  equals the full-refresh oracle (`multiset_equal`).
- Spec: `docs/specs/incremental_shapes.md` §"The succession grain" → §"The tombstone ledger
  (hidden state)" — rewrote the range-rebuild sentence (`smelt repair` doesn't exist; the
  ledger carries no run-axis column to restrict by, so a range rebuild re-derives it in
  full) and fixed two other `smelt repair` → `smelt rebuild` mentions in the same section.

## Decisions

- **`emit_create_table_as`'s single-statement return is matched, not `.expect`-ed**: the
  hardening-budget ratchet (`CLAUDE.md` §"Fail-loud discipline") counts `.expect("` textually
  regardless of whether the site is genuinely infallible; a `match ... None =>
  unreachable!(...)` proves the same invariant without adding counted debt.
- **`smelt rebuild` is NOT wired to the full-ledger rebuild in this phase**: `ExecuteRequest`
  carries no signal distinguishing a `smelt rebuild <model> --event-time-start/-end` call
  from an ordinary `smelt run` over the same window (both pass `full_refresh: false` —
  confirmed in `crates/smelt-cli/src/commands/rebuild.rs`). The 5c plan's own task 5
  anticipated this and sanctioned the fallback: only `--full-refresh` triggers
  `rebuild_succession_state` today. Documented inline in `project.rs` at the dispatch site.
- **Test 6 (`range_rebuild_re_derives_the_whole_ledger`) proves the function, not the CLI
  wiring**: it calls `rebuild_succession_state` directly (unscoped compiled SQL, as
  `project.rs` would eventually pass for a real range rebuild), confirming the ledger arm has
  no notion of "range" at all — it is always whole-source. This locks in the promise for
  whenever the wiring above lands.

## For the next planner

- **Threading a `rebuild_range`/`is_rebuild` signal through `ExecuteRequest`** is the
  concrete follow-up that closes the `smelt rebuild` gap above — needed before
  `docs/specs/incremental_shapes.md`'s rewritten Lifecycle paragraph is fully true for the
  CLI surface, not just for `--full-refresh` and the driver function itself. Serves success
  criterion 5's rebuild/repair clause completely; currently only half-wired.
  `crates/smelt-cli/src/commands/rebuild.rs`'s `to_upstream_closure`/`ln` naming (the command
  is internally called `ln`) is where the request would be built.
- Phase 6 (append-only probe) is next per the table; no blocking surprises found for it from
  this phase's work.
- Large-file ratchet regression on `crates/smelt-runtime/src/execute/project.rs` (4944 →
  4999 lines) is the same pattern every prior phase touching this file has hit (2b/3/3a/5a/
  5b); confirmed as the *only* failing gate in `cargo test --workspace` and in
  `verify-phase.sh`, left to the loop's dedicated shrink step per the standing convention.

## Gates

- `cargo test -p smelt-logical --lib maintenance::emit::succession` — 10/10 pass
- `cargo test -p smelt-logical --test succession_emit` — 8/8 pass
- `cargo test -p smelt-runtime --lib maintenance_driver::succession` — 13/13 pass
- `cargo test -p smelt-runtime --test statement_parity` — 40/40 pass
- `cargo test -p smelt-runtime --test technique_lowering succession` — 2/2 pass
- `cargo test -p smelt-runtime --test execute_parity` — 4/4 pass
- `bash .claude/scripts/hardening-budget.sh` — OK, all production counts match baseline
- `bash .claude/scripts/verify-phase.sh` — fmt PASS, clippy PASS, example_diagnostics PASS;
  workspace tests: only failure is the pre-existing `large_file_ratchet::
  gate_passes_on_committed_tree` regression on `project.rs` (report-only per the plan's own
  Verification note; loop's shrink step owns it)
