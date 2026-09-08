# Phase 3 summary — succession full-refresh folds on `(key, clock)`

**Shipped:**
- `emit_succession_full_rebuild` (`crates/smelt-logical/src/maintenance/emit/succession.rs`)
  now takes `output_columns`, `lead_derived`, `lag_derived` and emits a `ROW_NUMBER()`-ranked
  pick of one whole physical row per `(key_cols, clock_col)` group, in the model's own output
  column order, instead of a bare passthrough of the model's compiled `SELECT`.
- The rebuild executor (`crates/smelt-runtime/src/maintenance_driver/succession/execute.rs::
  rebuild_succession_state`) now runs the same clock-tie probe the patch loop runs, scoped to
  the whole source, before the presented write — refusing a content-disagreeing tie
  (`SuccessionClockTie`) rather than silently resolving it.
- `docs/specs/incremental_shapes.md` §"The tombstone ledger (hidden state)" — "Lifecycle"
  documents both: the `(k, t)` fold and the rebuild-path clock-tie probe.
- `crates/smelt-cli/tests/github_activity_oracle.rs`'s `DIVERGENCE_REGISTRY` no longer
  carries `silver_repo_naming`/`silver_actor_naming` (the `Bound::FoldEquality` variant was
  retired, now dead). `docs/handoffs/2026-09-08-github-activity-findings.md` and
  `examples/github_activity/README.md` updated to reflect the fix.
- New tests: `succession_emit.rs::full_rebuild_folds_on_key_and_clock`,
  `::full_rebuild_probes_clock_ties`, plus unit-level shape tests in `emit/succession.rs`.

**Decisions:**
- **Whole-row pick, not per-column aggregate.** A first implementation used `MAX()` per
  non-key/non-clock column, matching the plan's literal wording. Running the full 30-day
  oracle exposed a real bug: `LEAD`/`LAG` are computed over the model's own
  *physically-duplicated* source rows, so two rows tied at one `(k, t)` can carry genuinely
  different derived-column values — one row's `LEAD` sees its tied sibling as "next" (a
  same-`t` ordering artifact), the other correctly sees the true next event or `NULL`. `MAX`
  prefers the artifact over the correct `NULL` and mixes per-column maxes into a row no
  physical row ever held. Fixed by picking one whole physical row via
  `ROW_NUMBER() OVER (PARTITION BY key, clock ORDER BY <tie-break>)`, tie-breaking toward a
  row whose raw `{lead}`/`{lag}`-passthrough columns are NOT self-referencing (`col = clock`).
- **Column order must match the patch loop's bootstrap shell.** The model's own projection
  order need not be key-first (`silver.actor_naming` projects `actor_id, actor_login,
  created_at, ...`). An initial key-first `SELECT` broke every position-based `EXCEPT ALL`
  comparison against the patch-built presented table. `output_columns` now preserves the
  model's own order.
- Both bugs were caught only by running the plan's own full 30-day
  `every_window_matches_the_full_refresh_oracle` gate, not by the smaller unit/emitter tests —
  confirms this gate's value for this class of defect.

**For the next planner:**
- The tie-break's self-tie detection only covers `lead_derived`/`lag_derived` entries whose
  template is exactly `"{lead}"`/`"{lag}"` (raw passthrough). A model whose only derived
  columns are transformed (e.g. only `"{lead} IS NULL"`, no raw passthrough) would fall back
  to an arbitrary stable pick with no artifact-avoidance signal — safe for content-identical
  ties (verified by the clock-tie probe) but unexercised by any fixture. Not a known bug, just
  untested breadth.
- Punch-list items 2-4 (phases 4-6) are untouched by this phase.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-logical --test succession_emit` — 10 passed
- `cargo test -p smelt-runtime --test statement_parity` — 41 passed
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb` — 101 passed
- `cargo test -p smelt-cli --test github_activity_oracle --test github_activity_replay --features duckdb` — 26 passed, 1 ignored (unrelated)
- `python3 examples/github_activity/run_incremental.py` — 30-day replay succeeded
