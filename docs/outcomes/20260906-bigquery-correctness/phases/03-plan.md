# Phase 3 plan — succession full-refresh folds on `(key, clock)`

## Objective

`emit_succession_full_rebuild` re-runs the model's raw compiled `SELECT` with no
`(key, clock)` addressing, so `--full-refresh` keeps every physically duplicated
tied row while the incremental patch loop folds it — measured at 139 extra rows for
`silver.repo_naming` and 145 for `silver.actor_naming`. Fold the rebuild on
`(key_cols, clock_col)` with a deterministic aggregate over every other output
column, and run the clock-tie probe on this path (it never has), so a
content-disagreeing tie fails loudly instead of silently corrupting a refresh.
Advances criteria 3 (every fix gated) and 5 (two of the five registered divergences
become resolved rather than tolerated), and clears punch-list item 1.

## Spec delta

`docs/specs/incremental_shapes.md` §"The tombstone ledger (hidden state)" →
"Lifecycle" (the full-rebuild paragraph): state that a full rebuild presents one row
per `(key, clock)` — the same addressing the patch loop's `MERGE ... ON` uses — so
the two legs agree row-for-row, and that the clock-tie probe runs on the rebuild
path as it does on the patch path, refusing a content-disagreeing tie before any
write. Spec edit lands first, in the same commit.

## Design note

The fold needs the model's **output schema**, which `emit_succession_full_rebuild`
does not take today. `execute_succession_full_rebuild`
(`crates/smelt-runtime/src/maintenance_driver/succession/execute.rs`) already
resolves the clock column's type from that schema, so the columns are in hand at
the call site — thread them in as an explicit `payload_columns: &[String]`
parameter (the same shape `emit_succession_clock_tie_probe` already takes), not as
a `SELECT *`. `SELECT DISTINCT *` was tried on the spine and found insufficient:
`LEAD`/`LAG` over an un-deduped source gives tied rows *different* computed values,
so exact-row dedup catches only byte-identical duplicates (50 of 139). Keep the
existing DuckDB-only `assert!` — this phase does not widen the dialect.

## Tests

Red-green, in this order:

1. `succession_emit.rs::full_rebuild_folds_on_key_and_clock` — the emitted presented
   `CREATE TABLE ... AS` groups by `(key_cols, clock_col)` and aggregates every
   other output column; no bare `{model_select_sql}` passthrough remains.
2. `succession_emit.rs::full_rebuild_fold_is_identity_with_no_extra_columns` — a
   model whose output is exactly the key + clock still emits a well-formed single
   row per pair (no empty aggregate list).
3. `succession_emit.rs::full_rebuild_preserves_ledger_arm` — the tombstone
   `DELETE` + `INSERT` arm and the group's `transactional: true` are untouched.
4. `statement_parity/succession.rs` — existing fixtures updated to the folded shape
   and the per-family executed-vs-emitted parity leg still passes.
5. `github_activity_oracle.rs::full_refresh_matches_incremental_replay` (or the
   registry-driven comparison it feeds) — `silver_repo_naming` and
   `silver_actor_naming` now compare **equal**, with their `FoldEquality`
   `DIVERGENCE_REGISTRY` entries deleted; the registry's two-sided check must fail
   if a deleted entry's relation diverges again.
6. `succession_emit.rs::full_rebuild_probes_clock_ties` — the rebuild statement
   group carries (or the executor issues) the clock-tie probe before the presented
   write, and a content-disagreeing tie is refused rather than written.

## Tasks

1. Edit `docs/specs/incremental_shapes.md` per the spec delta.
2. Write tests 1-3 against the current emitter; watch them fail.
3. Add `payload_columns: &[String]` to `emit_succession_full_rebuild` and emit the
   `(key_cols, clock_col)` fold with a deterministic aggregate (`MAX`, matching the
   spine's recommendation) over each remaining output column.
4. Thread the output columns from `execute_succession_full_rebuild`'s already-resolved
   schema; update every other call site and fixture.
5. Run the clock-tie probe on the rebuild path before the presented write; refuse a
   disagreeing tie with the same error the patch path uses (test 6).
6. Update `statement_parity/succession.rs` fixtures (test 4).
7. Delete the two `FoldEquality` entries from `DIVERGENCE_REGISTRY` and confirm the
   oracle compares equal (test 5); update the handoff's divergence table row status
   and `examples/github_activity/README.md` if it restates the counts.
8. Re-run the 30-day replay to confirm the 139/145 deltas are gone.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test succession_emit`
- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-cli --test github_activity_oracle --test github_activity_replay`
- `python3 examples/github_activity/run_incremental.py`

No live warehouse is needed; every leg is DuckDB or emitted-SQL text.

## Commit message

`fix(succession): fold the full-refresh rebuild on (key, clock) and probe clock ties`
