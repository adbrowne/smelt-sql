# Phase 6a summary — Rebuild wiring

**Shipped:**
- `ExecuteRequest::rebuild: bool` (`crates/smelt-runtime/src/types.rs`), doc-commented as
  single-consumer: read only by the succession dispatch, ignored by every other grain.
- `crates/smelt-cli/src/commands/rebuild.rs`: pure `build_rebuild_request(...)` extracted from
  `rebuild()`, sets `rebuild: true`; unit test `rebuild_request_sets_the_rebuild_signal`.
- `crates/smelt-runtime/src/execute/project/mod.rs`'s succession dispatch (~2296–2360) widened
  to `request.full_refresh || force_full_refresh || request.rebuild`; comment rewritten to
  describe the wiring and cite the spec rather than "left to the next planner".
- Every other `ExecuteRequest` literal in the workspace (~40 sites in `smelt-cli`,
  `smelt-runtime`, `smelt-ui`, `smelt-maintenance-testkit`) gained `rebuild: false,` with no
  behaviour change — enumerated via `cargo check --workspace --all-targets`.
- `docs/specs/incremental_shapes.md` §"The tombstone ledger (hidden state)" — Lifecycle and
  Physical shape paragraphs corrected: a succession rebuild's range selects which models
  rebuild, never how much of one model's state is re-derived.
- New tests: `succession_patch_e2e.rs` (tests 2–4: rebuild re-derives the ledger, ignores the
  event-time window, and an ordinary run does not), `statement_parity/succession.rs`
  (`succession_rebuild_executed_statements_match_the_emitters`), `keyed_frontier_bookkeeping.rs`
  (`rebuild_signal_does_not_change_the_keyed_grain_path`).

**Decisions:**
- `request.rebuild` is single-consumer by design (verified via `rg -n "\.rebuild\b"
  crates/`) — see outcome.md Decision log 2026-09-07.
- `ui/src/types.ts`'s `RunExecuteRequest` left unchanged; the UI has no rebuild command.

**For the next planner:**
- Fixed two false-positive struct-literal edits caught by compile errors during the mechanical
  `rebuild: false,` sweep (a `clap` `RunArgs`/`RebuildArgs` field named `full_refresh: bool` in
  `main.rs`, a `MigrationPlan` literal, and a function parameter in `migrate_step.rs`) — worth
  a second pair of eyes if a future field addition does the same sweep.
- The large-file ratchet regressed by exactly 1 line on 5 files purely from the mandatory
  `rebuild: false,` field (`cross_midnight_rebase.rs`, `resume.rs`, `link_c_harness.rs`,
  `key_addressed_model_edge_lowering.rs`, `repair_lowering.rs`); baseline updated via
  `large-file-check.sh --update`. This same update also dropped a pre-existing orphaned
  baseline entry (`crates/smelt-deleted-crate/src/lib.rs`, already absent) — unrelated cleanup,
  not new scope.
- No follow-up work identified beyond the outcome's own phases 6b/6c already queued.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-runtime --test technique_lowering succession` — 5 passed
- `cargo test -p smelt-runtime --test statement_parity` — 41 passed
- `cargo test -p smelt-runtime --test execute_parity` — 4 passed
- `cargo test -p smelt-cli --bin smelt commands::rebuild` — 1 passed
- `bash .claude/scripts/hardening-budget.sh` — OK, matches baseline
