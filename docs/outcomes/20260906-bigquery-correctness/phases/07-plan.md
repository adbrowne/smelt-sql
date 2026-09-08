# Phase 7 — the repair edge from a Form-B upstream's self-rebase to its Form-A downstream

**Outcome:** `docs/outcomes/20260906-bigquery-correctness/outcome.md` (row 7)
**Advances:** criteria 2 (punch-list item 4, the last one), 3 (gated), 5 (registry → empty), 7.
**Handoff item:** `docs/handoffs/2026-09-08-github-activity-findings.md` §"Punch-list" item 4.

## Objective

`marts.daily_active_contributors` is Form A over `silver.actor_sessions` (it reads
`session_start_date` verbatim), so its run window is the requested window; but
`actor_sessions` is Form B and legitimately rewrites `[D−1, D+2)` on a `[D, D+1)` run. The
mart never revisits `D−1`, so `total_events` freezes at first-write time. Fix: a model's own
run window is widened to cover the **derived output window of every upstream maintained model
built in the same invocation**, before its own skew inversion is applied on top.

**First, the handoff's question, answered up front (task 1):** phases 4-5's mechanism does
**not** cover this. The edge here is clocked, so `append_model_edge_cells`' clock route
already derives a `NewData(silver.actor_sessions)` / `RecomputeRegion` / `DeleteInsert` cell
for the mart — the *cell exists*. The gap is that no run ever dispatches it over the
partitions the upstream actually rewrote, because the downstream's run window is the
invocation's requested window verbatim. Phases 4-5's route is key-addressed value enrichment
for a *clockless* upstream; this read is membership-sensitive (an aggregate) and clocked.
Confirm this by inspection (`smelt explain --json` on the mart) and record it in task 1's
notes — if the cell turns out to be absent, stop and re-plan rather than adding a second
mechanism.

## Spec delta (spec-first — the implement step makes these edits)

1. `docs/specs/model_transforms.md` §Semantics "The output window is derived, never assumed" —
   append a paragraph **"The derived output window propagates within a run."** A model's run
   window is the union of the requested run window and the derived output window of every
   upstream maintained model **selected in the same invocation**, aligned outward to the
   downstream's own granularity; the downstream's own skew inversion then applies to that
   union. Rationale in one sentence: without it a Form-B upstream's rebase of an earlier
   partition leaves a Form-A downstream that reads it verbatim frozen at first-write time,
   which is an equivalence violation, not a chunking artifact. State the two boundaries: an
   upstream *not* selected writes nothing and contributes nothing; a zero-skew upstream's
   output window equals the run window, so the union is a no-op (every existing fixture's
   literals are unchanged).
2. `docs/specs/incremental_models.md` §"Forward propagation — what must run" — one sentence
   cross-referencing the above: the same reflection an explicit landed delta gets
   (`[a, b)` → `[a − after, b + before)`) is applied to an ordinary windowed run's in-run
   upstream output windows, so the two entry points agree on which downstream partitions a
   Form-B rebase dirties.

## Tests (red-green)

New file `crates/smelt-runtime/tests/downstream_output_window_propagation.rs` unless noted.

1. `output_window_reports_the_batch_tiling_envelope` — `IncrementalWindows::output_window()`
   returns `(first.partition_start, last.partition_end)`, and `None` for an empty batch list.
2. `a_form_b_upstream_widens_a_form_a_downstream_run_window` — the pure widening helper:
   requested `[D, D+1)` ∪ upstream output `[D−1, D+2)` → `[D−1, D+2)`.
3. `an_upstream_window_inside_the_run_window_widens_nothing` — a zero-skew upstream leaves the
   requested window byte-identical (the guard for every existing fixture's literals).
4. `propagated_widening_aligns_outward_to_the_downstream_granularity` — a day-grained upstream
   window widening a month-grained downstream aligns out to month boundaries, so
   `validate_run_window_against_partition_grid` still accepts the result.
5. `a_mismatched_partition_axis_propagates_nothing` — a calendar upstream contributes nothing
   to an integer-axis downstream (points of different axes are never mixed); the case is
   warned, not silently dropped.
6. `a_form_a_downstream_runs_over_its_form_b_upstreams_rebased_window` — end-to-end through
   `execute_project` with `dry_run: true`, mirroring `dry_run_statements.rs`' harness
   (`PanicBackendFactory` + a `RecordingReporter` capturing `ChunkInfo`): a two-model synthetic
   project (Form-B upstream declaring a ±1-day relation, Form-A downstream reading it
   verbatim) run over `[D, D+1)` reports the downstream's chunk window as `[D−1, D+2)`.
7. `an_unselected_upstream_does_not_widen_its_downstream` — same harness with `select` naming
   only the downstream: the chunk window is the requested `[D, D+1)`.
8. `crates/smelt-cli/tests/github_activity_oracle.rs::marts_daily_active_contributors_matches_the_full_refresh_oracle`
   — the mart now matches the oracle on every column including `total_events`; written in the
   shape of the phase 5/6 sibling tests, with the `DIVERGENCE_REGISTRY` entry deleted (leaving
   it empty) and the const's doc comment rewritten to record the fix.

## Tasks

1. Answer the handoff's "does item 2's fix cover this?" question by inspection; record the
   answer (expected: no — cell exists, dispatch window is the gap) in the phase summary.
2. Add `IncrementalWindows::output_window()` (`crates/smelt-runtime/src/windowing.rs`) — test 1.
3. Add the pure widening helper in `windowing.rs` (requested window + upstream output windows +
   downstream granularity/axis → widened window, aligned outward, axis-matched) — tests 2-5.
4. Thread it through `build_model_plans` (`crates/smelt-runtime/src/execute/plan.rs`): carry a
   `model_name → output window` map across the topologically-ordered `selected` loop, widen the
   calendar `full_range` from this model's `refs` ∩ recorded upstreams **before** the
   `frozen_horizon` clamp (which narrows, never widens), and record this model's own output
   window after `compute_incremental_windows_ordered` returns — tests 6-7.
5. Make the spec edits above.
6. Run the `github_activity` oracle; delete the `DIVERGENCE_REGISTRY` entry and add test 8.
7. Update `docs/handoffs/2026-09-08-github-activity-findings.md`: root cause 4 gains a **Fixed
   by** paragraph, punch-list item 4 is marked Done, and the registered-divergence table becomes
   empty (say so explicitly rather than deleting the section).
8. Re-run `cargo test -p smelt-cli --test tutorial_freshness --features duckdb`. The
   `web_analytics` tutorial's downstreams of `silver/sessions.sql` are Form-A over a Form-B
   upstream, so their window literals are expected to move; regenerate with
   `python3 examples/web_analytics/generate_tutorial.py`, read the diff, and confirm each moved
   literal is the widened-by-skew window — do not accept a diff you cannot explain.
9. Update `examples/github_activity/README.md` §"Trusting the numbers" if it names the mart's
   divergence.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test downstream_output_window_propagation --test windowing_form_b_chunking --test windowing_parity --test windowing_ordered --test partition_axis_windowing --test statement_parity --test dry_run_statements --test since_upstream_propagation`
- `cargo test -p smelt-cli --test github_activity_oracle --features duckdb`
- `cargo test -p smelt-cli --test github_activity_replay --features duckdb`
- `cargo test -p smelt-cli --test tutorial_freshness --features duckdb`
- `cargo test -p smelt-lsp --test example_workspaces`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`fix(windowing): widen a run window to cover every in-run upstream's derived output window`
