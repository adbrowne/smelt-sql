# Handoff: execute the derived-output-window plan

**Branch**: `worktree-incremental` (worktree at `/home/andrew/smelt-sql/.claude/worktrees/incremental` — run everything from there, never `cd` to the main checkout).
**Plan**: `docs/plans/20260711-derived-output-window.md` (5 phases, all `pending`). Execute it with `/smelt:implement`, phase by phase, implementer subagent → reviewer subagent → commit+push per phase.
**Spec (correctness oracle)**: commit `b7c2d270` — `docs/specs/model_transforms.md` §"The output window is derived, never assumed" + §Design "Derived output window composes with chunking"; `docs/specs/batched_models.md` §"Execution model" items 1–2. Spec decisions are settled; do not re-open.

## What this fixes (context you'd otherwise have to re-derive)

A run's DELETE range and `_smelt_output_clamp` are both built from the batch's run window verbatim (`crates/smelt-runtime/src/execute.rs`, `PartitionRange` + `derive_batch_filtered_sql`). A model whose `partition_column` is derived and skews from the driving date column — declared by a Form B relation like `event_date BETWEEN session_start_date - INTERVAL '1 day' AND session_start_date + INTERVAL '1 day'` — silently under-writes: the scan IS widened, the correct neighbour-partition row IS computed, then the clamp discards it, and no later run's window contains that partition.

Deterministic repro (already verified, don't redo unless useful): `examples/web_analytics`, datagen seed 42 scale 0.01, day-by-day replay 2026-03-19→2026-05-08, then
`smelt run --select silver.sessions --event-time-start 2026-05-04 --event-time-end 2026-05-05 -v`
leaves session `1448-2026-05-03 23:47:30` at `event_count=1` even though event 7647 (2026-05-04 00:03:36, same device, 16-min gap) is in `silver_events_parsed`. Full root-cause writeup: `docs/plans/20260710-web-analytics-maintenance-demo.md` §"Deferred during implementation".

## Key facts for the implementer

- **The fix seam is one place**: `crates/smelt-runtime/src/windowing.rs::compute_incremental_windows` — derive `output_range = [start − after, end + before)` from the skew bound and chunk THAT; `filter_start/filter_end` and the execute loop's DELETE/clamp are already batch-relative, so they inherit correctness.
- **Inversion direction** (easy to get backwards): relation `driving_date ∈ [p − before, p + after]` inverts to output window `[start − after, end + before)` — the `+ after` side extends the window *earlier*.
- **Skew derivation must be pure in `smelt-logical`** (`analysis/source_bounds.rs`, mirror of `extract_form_b_bounds` but matching the *anchor* side = model's own partition column) and invoked as a walk leaf classifier — maintenance-plan purity + property-composition-walk invariants in root `CLAUDE.md` apply.
- **Transparent fast path** (`is_transparent_single_source`, `transformer.rs`) must require zero skew, else the outer clamp is dropped for write-rebasing models.
- The Rust harness misses this bug class because it asserts only `(session_id, utm_campaign)` — both invariant (session identity = root timestamp; attribution = first 5 min). Phase 3 strengthens it; verify the strengthened assertion is red on the pre-fix runtime.
- `smelt explain --show-sql` fails clamp injection on function-at-FROM models ("No FROM clause found") because the emission branch gets unexpanded SQL; the live run expands first. Phase 4 fixes ordering; statement-parity gate is the oracle.

## Environment (every cargo/smelt command)

```
export DUCKDB_LIB_DIR=~/.local/lib/duckdb
export LD_LIBRARY_PATH=~/.local/lib/duckdb:$LD_LIBRARY_PATH
```
Binary at `target/debug/smelt` in the worktree (built at HEAD). `smelt-datagen` on PATH (`~/.cargo/bin`). Set `CARGO_INCREMENTAL=0` if `/dev/shm` fills. Verification gate: `bash .claude/scripts/verify-phase.sh` (one command, not four).

## Definition of done

All 5 phases `done` in the plan's Progress table, then the plan's §Verification: 60-day `verify_incremental_equivalence.py` passes (day-46 divergence closed), strengthened `per_partition_equivalence` green, `statement_parity` green, verify-phase.sh clean, `/smelt:validate model_transforms` + `batched_models` zero drift on the output-window sections.
