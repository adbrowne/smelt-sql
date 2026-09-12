# Phase 6 plan — interior-chunk forward reach for Form-B models

## Objective

`compute_calendar_windows` widens each batch's *filter* (scan) range only by the
SQL-derived lookback/lookahead, never by the model's derived partition-column skew, so an
interior chunk boundary truncates a Form-B model's forward reach: the chunk writes
partitions `[bs, be)` from a scan that stops at `be`, losing the driving-date rows in
`[be, be + after]` that those partitions' own declared relation admits. Fixing it closes
punch-list item 3 (criterion 2), removes the `silver_actor_sessions` `DIVERGENCE_REGISTRY`
entry in favour of exact equality (criterion 5), and lands an offline chunk-invariance gate
(criterion 3).

## The defect, precisely

Form B declares `driving_date BETWEEN partition_column − before AND partition_column + after`
(`Skew { before, after }`). Two inversions fall out of it, and only the first is implemented:

- **Write side (implemented).** Run window `[start, end)` → output window
  `[start − after, end + before)`. Applied once, to the whole invocation.
- **Scan side (missing).** To compute partitions `[bs, be)` the scan must cover driving dates
  `[bs − before, be + after)`. Today `filter_start = bs − lookback`, `filter_end = be + lookahead`.
  For a single-chunk invocation the batch bounds *are* the output-window bounds, so the write-side
  widening incidentally covers it; at every interior boundary it does not.

Fix: fold the skew into the per-batch filter, **clamped to the invocation's existing outer scan
envelope** so single-chunk literals stay byte-identical and only interior batches change —

```
filter_start = max(bs − lookback − before_days, output_start − lookback)
filter_end   = min(be + lookahead + after_days,  output_end   + lookahead)
```

`partition_start`/`partition_end` (the DELETE range and the output clamp) are untouched. The
integer axis needs no change — it already refuses a nonzero skew fail-closed.

## Spec delta

The spec is already normative here and the code contradicts it: `docs/specs/incremental_shapes.md`
§"Execution model (DuckDB)" says chunking splits the output window into pairs "each sized from its
own chunk's reach", and `docs/specs/model_transforms.md` §Semantics "The output window is derived,
never assumed" says the widened scan is sized "relative to the derived output window", never the run
window. Add one clarifying sentence to each, stating that a chunk's own reach is the **skew
inversion of that chunk's partition range** (`[bs − before, be + after)`), not only the frame margin
`k`, and naming the resulting property: a run's written output is invariant under chunk count
(`--batch-size` / batch-safety sizing changes performance, never results).

## Tests (red first)

New file `crates/smelt-runtime/tests/windowing_form_b_chunking.rs` (both `windowing.rs` and
`windowing_parity.rs` sit exactly at their large-file baselines — do not grow the latter):

1. `interior_chunk_scan_carries_the_form_b_forward_reach` — a Form-B model (`WHERE driving BETWEEN
   part_col AND part_col + INTERVAL '1 day'`, cf. `examples/github_activity/models/silver/actor_sessions.sql`)
   over a range chunked into ≥3 batches: every interior batch's `filter_end ≥ partition_end + after`
   and `filter_start ≤ partition_start − before`. RED today on `filter_end`.
2. `outer_scan_envelope_is_unchanged_by_the_interior_widening` — first batch's `filter_start` and
   last batch's `filter_end` equal today's values (the clamp), so no existing statement literal moves.
3. `chunking_is_invariant_for_a_form_b_model` — the same run window computed with `batch_size_days`
   of 1, 3 and `None` yields the same partition cover, and every batch in every sizing satisfies
   test 1's reach predicate.
4. `zero_skew_model_batches_are_byte_identical` — an identity (Form-A) model's batches are unchanged
   in all four fields, pinning that the fix is skew-gated.
5. `crates/smelt-cli/tests/github_activity_oracle.rs::silver_actor_sessions_matches_the_full_refresh_oracle`
   — exact equality of the incremental and full-refresh legs on that relation (the pattern of
   `gold_events_enriched_matches_the_full_refresh_oracle`), after deleting the relation's
   `DIVERGENCE_REGISTRY` entry.

## Tasks

1. Write tests 1–4 red against the current `compute_calendar_windows`.
2. Apply the clamped per-batch skew widening in `compute_calendar_windows`' tiling loop; green 1–4.
3. Make the spec edits above (both files), then the doc-comment on `compute_calendar_windows` /
   `compute_incremental_windows` naming the scan-side inversion alongside the write-side one.
4. Delete the `silver_actor_sessions` entry from `DIVERGENCE_REGISTRY` (and prune the corresponding
   paragraph of its doc comment), add test 5, and run the full 30-day oracle.
5. Update `docs/handoffs/2026-09-08-github-activity-findings.md`: root cause 3 gains a **Fixed by**
   paragraph and punch-list item 3 becomes **Done**, following the convention root causes 1 and 2 set;
   drop the `silver_actor_sessions` row from the registered-divergence table. The handoff-sync tests
   (`every_registry_entry_is_named_in_the_findings_handoff`, `findings_handoff_names_no_unknown_relation`)
   must stay green.
6. Re-measure `marts_daily_active_contributors`' divergence — this fix changes what `actor_sessions`
   writes, so that entry's `monotone_columns`/direction may shift. Do **not** fix it (phase 7 owns it);
   if its registered predicate no longer holds, update the entry's numbers/reason and say so in the
   summary as phase 7's new starting point.
7. If `windowing.rs` exceeds its 1173-line baseline, bump `.claude/large-file-baseline.txt` with a
   one-line sign-off note (phase 5 precedent) rather than leaving the ratchet red.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test windowing_form_b_chunking --test windowing_parity --test windowing_ordered --test partition_axis_windowing --test statement_parity --test dry_run_statements`
- `cargo test -p smelt-cli --test github_activity_oracle --test github_activity_replay --features duckdb`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`fix(windowing): size every chunk's scan from its own Form-B forward reach`
