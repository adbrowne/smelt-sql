# Phase 6a plan — Rebuild wiring: `smelt rebuild` takes the succession full-ledger path

## Objective

Thread a rebuild signal through `ExecuteRequest` so that `smelt rebuild <model>
--event-time-start/-end` on a succession model takes `rebuild_succession_state` (presented
table + tombstone ledger re-derived in one transaction) instead of the window-forward patch
loop. Today only `--full-refresh` reaches that path, so criterion 5's "`--full-refresh` and
`smelt repair` rebuild the ledger" clause is half-wired and
`incremental_shapes.md` §"The tombstone ledger (hidden state)" — Lifecycle is untrue of the
CLI surface. Closes the follow-up phase 5c's summary named.

## Spec delta

`docs/specs/incremental_shapes.md` §"The succession grain" → §"The tombstone ledger (hidden
state)", the **Lifecycle** paragraph. Today it says a range rebuild re-derives the ledger in
full "in the same transaction as that range's presented rebuild", implying the *presented*
rebuild is range-restricted. For this grain it is not: neighbour relationships cross window
boundaries and the presented table carries no run-axis partitioning, so
`rebuild_succession_state` re-derives presented rows and ledger both from the whole source.
Edit that sentence to say so — a `smelt rebuild` of a succession model uses its range to
select *which* models rebuild, never to narrow the rebuilt extent of one — and mirror the
one-line consequence in the §"Physical shape" paragraph's "rebuilt in the same transaction on
`--full-refresh` and `smelt rebuild`" clause if it reads as range-restricted. Timeless-oracle
rule applies: no phase vocabulary. Do not touch §Known Divergences (phase 10 owns it).

## Tests

Red-green, in this order:

1. `crates/smelt-cli/src/commands/rebuild.rs` (new `#[cfg(test)] mod tests`, declared per the
   phase-3b file-selection rule) — `rebuild_request_sets_the_rebuild_signal`: the extracted
   pure `build_rebuild_request(...)` returns `rebuild: true`, `full_refresh: false`, and the
   upstream-closure selectors. Red: no such field/function.
2. `crates/smelt-runtime/tests/technique_lowering/succession_patch_e2e.rs` —
   `rebuild_request_re_derives_the_tombstone_ledger`: drive two windows normally, then insert
   a bogus `(k, t)` row into `<table>__tombstones` directly, then run with `rebuild: true`
   over one window; the bogus row is gone and the presented table equals the model SQL's
   full-refresh oracle. Red today (window-forward path leaves it).
3. Same file — `rebuild_ignores_the_event_time_window`: after two windows, drop the presented
   table's rows for window 1 out-of-band, run with `rebuild: true` over window 2 only, and
   assert window 1's rows are back — the rebuild extent is the whole source, not the range.
4. Same file — `an_ordinary_run_over_the_same_window_still_patches`: the identical request
   with `rebuild: false` leaves the bogus tombstone row in place (proves tests 2/3 discriminate
   the signal, not just the re-run).
5. `crates/smelt-runtime/tests/statement_parity/succession.rs` —
   `succession_rebuild_executed_statements_match_the_emitters`: mirror of the existing
   `succession_full_refresh_...` leg with `request.rebuild = true` (and `full_refresh: false`);
   executed statements equal `emit_succession_full_rebuild`'s group.
6. `crates/smelt-runtime/tests/keyed_frontier_bookkeeping.rs` (or nearest keyed e2e) —
   `rebuild_signal_does_not_change_the_keyed_grain_path`: a keyed-grain model run with
   `rebuild: true` records the same strategy/time range as with `rebuild: false` — the signal
   is scoped to the succession dispatch and changes nothing else.

## Tasks

1. Spec edit above, first.
2. Add `#[serde(default)] pub rebuild: bool` to `ExecuteRequest`
   (`crates/smelt-runtime/src/types.rs`), doc-commented as: set by `smelt rebuild`; consumed
   only by the succession dispatch, where it means re-derive presented state and ledger in
   full; ignored by every other grain.
3. Fix the fallout mechanically — every exhaustive `ExecuteRequest` literal in the workspace
   (~50 sites across `smelt-cli`, `smelt-runtime`, `smelt-ui`, `smelt-maintenance-testkit`)
   gains `rebuild: false,`. Use `cargo check --workspace --all-targets 2>&1 | tail -50` as the
   enumerator; no behaviour change at any site.
4. Extract `build_rebuild_request(args: &RebuildArgs, upstream_selectors: Vec<String>,
   ephemeral_seed_ctes: …) -> ExecuteRequest` out of `commands::rebuild::rebuild`, setting
   `rebuild: true`; call it from `rebuild()`. Test 1 targets this function.
5. In `crates/smelt-runtime/src/execute/project/mod.rs`'s succession dispatch (~2296–2380),
   widen the branch to `request.full_refresh || force_full_refresh || request.rebuild` and
   replace the "left to the next planner" comment block with what the wiring now does and why
   the range is not honoured (cite the spec section, not this phase).
6. Confirm nothing else reads `request.rebuild` (`rg -n "\.rebuild\b" crates/`), keeping the
   signal single-consumer.
7. If `ui/src/types.ts`'s `RunExecuteRequest` is unchanged (it is a subset and the UI has no
   rebuild command), leave it alone and say so in the summary.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test technique_lowering succession --quiet 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test statement_parity --quiet 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test execute_parity --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --bin smelt commands::rebuild --quiet 2>&1 | tail -20` (the module lives in the binary crate, not the lib)
- `bash .claude/scripts/hardening-budget.sh`
- Report (do not silently absorb) any large-file ratchet regression on
  `execute/project/mod.rs`; the loop's shrink step owns it.

## Commit message

`feat(succession): route smelt rebuild through the full-ledger rebuild path`
