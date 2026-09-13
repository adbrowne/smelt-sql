# Phase 9e summary — the ledger-free succession full rebuild

## Shipped

- `docs/specs/state.md` §"The degradation contract": the succession-grain paragraph now states
  exactly what the downgraded rebuild writes and skips (presented arm alone, no tombstone
  relation, no clock-tie probe; a `(k, t)` tie resolved by the fold's own deterministic
  tie-break instead of being refused ahead of time).
- `crates/smelt-logical/src/maintenance/emit/succession/mod.rs`: extracted the presented-arm
  fold (`CREATE TABLE … AS`) shared by both full-rebuild emitters into a private
  `presented_arm_statement` helper, so the two paths cannot drift in fold shape or column
  order. Added `pub fn emit_succession_full_rebuild_ledgerless(...)` beside
  `emit_succession_full_rebuild`: infallible, dialect-blind, returns a single non-transactional
  statement (the presented arm only) — the single owner of the state-downgraded rebuild's
  statement.
- `crates/smelt-runtime/src/maintenance_driver/succession/execute.rs`:
  `rebuild_succession_state` now branches on `cell.state_downgraded` **before** the
  `realises_tombstone_ledger` guard. The downgraded arm drops the presented table, calls the
  new ledgerless emitter, reports the one statement, and executes it via
  `retry_backend_call`/`execute_write_with_bookkeeping` with empty ensure/cleanup lists —
  skipping the ensure-DDL, presented shell, clock-tie probe and ledger statements entirely. The
  existing `realises_tombstone_ledger` refusal stays as the backstop for a non-downgraded cell
  on a no-ledger dialect.
- `crates/smelt-runtime/tests/succession_downgraded_rebuild.rs` (new): executes the downgraded
  path against a real DuckDB backend (not just the dispatch decision) — verifies the rebuild
  succeeds and matches the full-refresh oracle, no tombstone table is created, exactly one
  statement is reported, the non-downgraded backstop still works and still creates the ledger,
  and re-running twice is idempotent.
- Updated the deferral comment at `crates/smelt-cli/tests/github_activity_dual_target.rs:1115`
  to describe the fix and point restoration of the report gates at phase 9f (not this phase —
  `08-parity.json` still does not exist).

## Decisions

- Kept the branch keyed on `cell.state_downgraded` (a plan-derived boolean), never on dialect —
  per `CLAUDE.md` §"Fail-loud discipline" and the `state_guard_census` structural gate, and
  because it is what makes the downgraded path executable and testable against a real DuckDB
  backend rather than needing a synthetic no-ledger dialect.
- `presented_arm_statement` takes `#[allow(clippy::too_many_arguments)]` like its two callers —
  matching this module's existing convention rather than introducing a parameter-object
  abstraction for one internal helper.
- The new emitter is dialect-blind and infallible by design: it never calls
  `check_succession_dialect`, since "does this backend realise the tombstone ledger" is not a
  question it needs answered — the caller's `state_downgraded` branch is the only gate.

## For the next planner

- Phase 9f can now resume 9d's live replay unchanged: reset/replay the dogfood state, run the
  full refresh and windows 9/10/11, re-run both sweeps, and restore the report-driven gates
  (`08-parity.json`, `09b-equivalence.json`, `dbx_registry_entries_are_all_live`).
- Nothing left the outcome; nothing added to `## Out of scope`.

## Gates

- `cargo test -p smelt-logical --lib maintenance::emit::succession` — 24 passed
- `cargo test -p smelt-logical --test succession_emit` — 10 passed
- `cargo test -p smelt-logical --test walk_coverage` — 14 passed
- `cargo test -p smelt-runtime --test succession_downgraded_rebuild` — 4 passed
- `cargo test -p smelt-runtime --test state_guard_census --test statement_parity --test execute_parity` — 49 passed
- `cargo test -p smelt-runtime --test technique_lowering` — 37 passed
- `bash .claude/scripts/verify-phase.sh` — VERIFY: ALL GREEN
