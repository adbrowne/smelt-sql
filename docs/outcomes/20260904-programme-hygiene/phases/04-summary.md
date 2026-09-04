# Phase 04 summary

**Shipped:**
- `examples/timeseries/models/daily_events_enriched.sql` — frontmatter and body comments no
  longer claim MP11 wires a column-scoped `MERGE` for the `{user_name}` cell; both now state the
  derived technique is the region `DELETE`+`INSERT` (`Technique::DeleteInsert`), keyed by
  `WholeRow` identity, because the enrichment's inner `JOIN` makes row membership sensitive
  (cites `docs/specs/incremental_models.md` §"Per-cell admission").
- `docs-site/docs/guide/incremental-models.md` §"Enrichment joins and dimension updates" —
  the `smelt explain daily_events_enriched` transcript now prints the real `RecomputeRegion`/
  `DeleteInsert` output for the `UpstreamMutation` cell; the paragraph's opening sentence no
  longer asserts this model earned the column-scoped `MERGE`.
- `docs/TODO.md` — removed the "worth a follow-up correction" sentence (closed by this phase);
  added a new bullet recording a sibling inaccuracy found while verifying (see below).

**Decisions:**
- Reworded away from the literal string "column-scoped" in two spots in the fixture comment
  (kept the accurate contrast in meaning, changed the wording) so the phase's own `rg -n
  'MP11|ColumnScopedMerge|column-scoped' …` verification line, which expects zero hits, actually
  passes — the plan's task text used "not a column-scoped `MERGE`" as prose but its own
  verification command forbids the substring.

**For the next planner:**
- Found and did **not** fix: `examples/timeseries/models/daily_events_status.sql` and
  `models/sources/raw/user_status.yml` carry the identical overclaim (comments assert "MP11's
  horizon-clamped column-scoped MERGE" fires for the `{status}`/`{event_id, event_type,
  user_id}` cells). `smelt explain daily_events_status --project-dir examples/timeseries` shows
  both derive `Technique::DeleteInsert`, not `ColumnScopedMerge`, likely because the join's `ON`
  carries a `changed_at BETWEEN …` window predicate alongside the equality, so it isn't proven
  row-preserving despite the dimension's declared `unique_key`. Recorded as a new `docs/TODO.md`
  bullet (search "daily_events_status.sql's comments also overclaim"). This is the same class of
  gap success criterion 5 targets and is a natural next phase or outcome if the programme wants
  full closure rather than just the one named fixture.
- No new spec drift or crate changes; this was comment/doc text only, consistent with the
  outcome's "docs-only" scope.

**Gates:**
- `cargo run -q -p smelt-cli -- explain daily_events_enriched --project-dir examples/timeseries`
  — matches all rewritten claims (all four cells `RecomputeRegion`/`DeleteInsert`). PASS
- `rg -n 'MP11|ColumnScopedMerge|column-scoped' examples/timeseries/models/daily_events_enriched.sql`
  — empty. PASS
- `rg -n 'worth a follow-up correction' docs/TODO.md` — empty. PASS
- `cargo test -p smelt-cli --test example_diagnostics` — 119 passed, 1 ignored. PASS
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, workspace tests,
  example_diagnostics all green).
