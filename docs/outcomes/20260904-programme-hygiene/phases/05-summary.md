# Phase 05 summary — correct `daily_events_status`'s overclaimed column-scoped MERGE

**Shipped:**
- `examples/timeseries/models/daily_events_status.sql`: frontmatter and body comments no
  longer claim "MP11's horizon-clamped column-scoped `MERGE`" / F15 dispatch; both now
  state the derived `RecomputeRegion`/`Technique::DeleteInsert` corner, citing
  `docs/specs/incremental_models.md` §"Per-cell admission". The genuine
  `PartitionLocal::Yes` / `ScanClamp` / clocked-dimension contrast with
  `daily_events_enriched.sql` is preserved verbatim in meaning.
- `examples/timeseries/models/sources/raw/user_status.yml`: `unique_key:` rationale
  rewritten — the key licenses the non-fan-out proof the region recompute needs;
  states the derived technique is `DeleteInsert`, not a per-column-targeted merge.
- `docs/TODO.md`: the `daily_events_status.sql`'s comments also overclaim
  `ColumnScopedMerge`" bullet (formerly ~:489-504) deleted.

**Decisions:**
- Confirmed via `cargo run -q -p smelt-cli -- explain daily_events_status --project-dir
  examples/timeseries` before editing: both `UpstreamMutation` cells derive
  `RecomputeRegion`/`DeleteInsert`, region key `WholeRow`, `locality: partition_local`,
  with a real `ScanClamp` on `raw.user_status.changed_at`. Matched the plan's ground
  truth exactly — no surprises.
- Avoided the literal substring `column_scoped` even in a "not a ... merge" sentence
  in `user_status.yml`, since the verification `rg` forbids the substring regardless of
  polarity; reworded to "per-column-targeted merge" instead.

**For the next planner:**
- No new gaps surfaced while editing (task 7's fallback was not needed).
- The dangling `§"What the composed shape uniquely enables"` citation gap from phase 2's
  summary remains open and untouched, as before.
- Phase 6 (validation: `/smelt:validate state` + `model_properties`, verify-phase) is the
  only remaining row.

**Gates:**
- `rg -n 'MP11|F15|ColumnScopedMerge|column.scoped|column_scoped' examples/timeseries/models/daily_events_status.sql examples/timeseries/models/sources/raw/user_status.yml` — empty. PASS
- `rg -n "daily_events_status.sql.s comments also overclaim" docs/TODO.md` — empty. PASS
- `cargo test -p smelt-cli --test explain_model` — 27 passed. PASS
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN. PASS
