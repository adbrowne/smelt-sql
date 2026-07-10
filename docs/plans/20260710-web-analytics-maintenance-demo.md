# DRAFT — Web-Analytics Maintenance Demo + Keyed Temporal Locality

**Status:** DRAFT / brainstorming output. Not yet a phased implementation plan.
**Date:** 2026-07-10
**Author:** Andrew (via brainstorming session)

> This document captures (a) the demo we want to build, (b) what was verified
> about the current implementation status of the features it needs, and (c) the
> decisions and open questions. It is intentionally paused: the next step is a
> **separate review session** to confirm the recent maintenance-plan (MP-series)
> work landed complete specs + code, after which we revisit and turn this into a
> real phased plan (`/smelt:plan`).

---

## 1. The goal — a realistic web-analytics maintenance demo

Extend `examples/web_analytics/` (chosen: **extend in place**, not a new example) into
a realistic-ish pipeline that shows off the model-maintenance features, then turn
it into a **script-generated tutorial** in the user docs whose SQL snippets are the
**real** insert/update/merge statements smelt emits (so the doc cannot drift).

### Intended pipeline

- **Bronze (source / landing)** — raw append-only events. Each event carries **two
  timestamps**: `event_time` (client-side occurrence) and `arrival_time` (when it
  landed). Conceptually partitioned by **arrival date** (the ingest axis).
  - *Caveat found:* in DuckDB, bronze is a passthrough **view** over the source, so
    "partitioned by arrival" is largely **narrative** — there is no physical
    arrival-partition to demonstrate. Keep `arrival_time` as a real column (drives
    dedup ordering + the lateness story); treat arrival-partitioning as conceptual.

- **Silver (`events`, cleaned + deduplicated)** — incremental ingest from bronze.
  - Accepts **late-arriving** events up to a **3-day** window (UTC day boundary).
  - **Deduplicates `event_id`** over that same window (re-delivery safety).
  - **Re-partitioned by event date** (not arrival date) — the analytical grain. The
    load-bearing repartition to demonstrate. A late event landing today can rewrite
    an event-date partition up to 3 days old = the bounded partition-level update.

- **Sessions (`sessions`, derived)** — group silver events into standard
  sessionization windows per visitor, with a **max session length** cap (upper
  bound → old partitions seal → safe partition-level maintenance). Capture
  **`utm_campaign`** from an event in the **first 5 minutes** of the session as a
  session attribute (campaign attribution). *(The sessionize showcase already
  exists in web_analytics; extend it with the campaign attribute + max-length cap
  if not already present.)*

- **Enriched silver (`events_enriched`)** — join `session_id` + campaign
  attribution back onto the event grain. Demonstrates that smelt targets a **narrow
  update** (only affected event-date/session partitions) rather than a full rebuild.

### Deliverable

1. The extended, **buildable** `examples/web_analytics/` workspace (kept green in
   `example_diagnostics` + the equivalence-proof harness).
2. A **script-generated tutorial** under `docs-site/docs/examples/` (or
   `getting-started/`) embedding the **real** emitted maintenance SQL.

---

## 2. Verified implementation status (as of HEAD, worktree-incremental)

Verified against **production code + tests**, not just spec prose (specs use a
"timeless-oracle" rule where true status lives in §Known Divergences and can lag).

### Ships today (mature path)
- **Partition-grain incremental** (`grain: partition` + `timeseries:` block):
  DELETE+INSERT per partition, derived lookback / scan clamp, batch-safety
  classification + auto-chunked backfill (`smelt backbuild`), safety-check
  overrides. Driven by `smelt run/build/backbuild --event-time-start/--event-time-end`.
- **Key-grain additive + extremal folds** (`grain: key`): `COUNT`/`SUM`/`BIT_XOR`
  and `MIN`/`MAX`/`BOOL_*`/`BIT_AND/OR` — a real DELETE-free MERGE path. `unique_key`
  = GROUP BY list; combiners derived from aggregators.
- **One column-scoped MERGE** demo path (`examples/timeseries/models/daily_events_enriched.sql`,
  `maintenance: scan_bounds.per_source.<src>.allow_full_scan: true`).
- **Forward propagation / backward resolution** graph CLI:
  `smelt run --since-upstream --source <addr> --landed <a>..<b>` and
  `smelt build <model> --period <a>..<b> --include-upstreams` (both refuse keyed
  nodes, cycles, self-refs; delta detection is **manual/explicit** — caller supplies
  `--landed`, no auto watermark diffing).

### NOT shipped / spec-only / refused — **directly blocks the literal demo spec**
- **Keyed dedup over an event-time window** — `grain: key` **+** `timeseries:` block
  is **refused unconditionally, today**, via `KeyedForbidsTimeseries`.
  - Production guard: `crates/smelt-core/src/metadata.rs:537`
    (`if metadata.is_keyed() && metadata.timeseries.is_some() { return Err(..) }`).
  - Refusal covered at 3 layers: `crates/smelt-core/tests/refresh_axis.rs`
    (`refresh_keyed_forbids_timeseries`), a `metadata.rs` unit test, and
    `example_diagnostics` BUG-006 + fixture
    `examples/timeseries_broken_cumulative_with_timeseries/`.
  - The "key temporal locality" routing that would relax it
    (`establish_locality`, the three routes, `KeyedRecurrenceBoundViolated`) **does
    not exist in code** — spec-only.
  - `keyed_models.md §Known Divergences:293` is **accurate** (matches production).
- **"Latest-copy-wins" / first-seen dedup folds** — `MAX_BY`/`MIN_BY`,
  `COALESCE`-first, `ANY_VALUE`, and the `smelt.latest()/once()/current()` sugar —
  **spec-only**.
- **Late-arriving-data automation** (per-column `data_latency:`) — **not
  implemented**. Only manual mitigation: trail `--event-time-end` behind real time,
  or re-run overlapping ranges.
- **Maintenance SQL is not printed by any command.** `smelt run --dry-run` prints
  only the compiled **SELECT body**, not the `DELETE+INSERT`/`MERGE` statements.
  Those are built by pure fns in `crates/smelt-logical/src/maintenance/emit.rs`
  (`emit_delete_insert:64`, `emit_column_scoped_merge:81`, `emit_in_place_update:111`,
  `emit_keyed_fold:133`) but executed, never surfaced. `smelt explain` prints the
  maintenance *plan* (cells/techniques/clamps), **not** literal SQL. **No golden-SQL
  artifact exists** to lift into docs.

### Doc drift found (fix regardless of this demo)
User-facing docs promise a **conditional** keyed+timeseries admission ("*unless / only
when key temporal locality is established*") that has **no code path** — production
refuses **unconditionally**:
- `docs-site/docs/guide/materializations.md:196`
- `docs-site/docs/reference/timeseries.md:85`
- `docs-site/docs/reference/cumulative-aggregate.md:95`
Minor spec/impl drift too: `keyed_models.md` claims the diagnostic message "names the
three routes"; the shipped message (`metadata.rs:421`) is the older blanket wording.

---

## 3. Decisions made in this session

1. **Extend `examples/web_analytics/` in place** (not a new example). It already
   implements ~80% of the pipeline: `bronze/raw_events.sql`,
   `silver/events_parsed.sql` (partition-grain, event_date), `silver/sessions.sql`
   (sessionize showcase, bounded lookback), `silver/device_user_edges.sql`
   (`grain: key` cumulative merge), `run_incremental.py` day-by-day driver +
   `verify_incremental_equivalence.py` full-rebuild-vs-incremental equality proof.
2. **Add a maintenance-SQL emission surface first** (extend `--dry-run` or add
   `smelt explain --show-sql`) so the tutorial captures real emitted MERGE/INSERT
   SQL — keeps the doc drift-proof. Chosen over a codegen-only script.
3. **Build keyed temporal locality first** (user decision) so silver dedup uses the
   *real* keyed-dedup-over-window path rather than the partition-grain reframe.
   - **Alternative on the table (revisit):** partition-grain reframe — express silver
     dedup as `grain: partition` on `event_date` with
     `QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1`;
     the 3-day window becomes the derived lookback / batch-safety bound. Same
     user-facing story, **builds today**, equivalence-verifiable. Kept as the
     fallback if keyed temporal locality proves too large.

---

## 4. Stacked work (each spec-first) — to be turned into real plans

1. **Key temporal locality** (feature, gates the demo's dedup story). Spec diff to
   `docs/specs/keyed_models.md` first: define what "locality established" means, the
   three routes, `key_recurrence` bound, dedup/latest-wins semantics, ledger
   idempotency (never double-fold a redelivery), the lowering technique (windowed
   keyed-fold MERGE within a clamped region), and refusal diagnostics. Then
   un-refuse `KeyedForbidsTimeseries` under real admission, migrate the 4 refusal
   tests + the broken fixture.
   - *(Spec-extraction of the full envisioned design was started then paused; resume
     in the revisit session.)*
2. **Maintenance-SQL emission surface** (`--dry-run`/`explain --show-sql` prints real
   `emit.rs` output). Reusable, drift-proofs the tutorial.
3. **web_analytics extension + generated tutorial** + the doc-drift cleanup (§2).

---

## 5. Open questions — resolve in the revisit session

- **Did the recent MP-series work actually leave keyed temporal locality
  un-built?** (This session verified: yes, refused unconditionally at HEAD. The
  review session should confirm the specs + code for the *shipped* MP work are
  complete/consistent before we layer locality on top.)
- **Is the full locality design in `keyed_models.md` implementable as written, or
  under-specified?** (The extraction agent was stopped before reporting — rerun it,
  or brief directly.)
- **Scope call:** build keyed temporal locality (large, multi-phase) vs. the
  partition-grain reframe (ships now) for the *demo*. If locality slips, the demo can
  ship on the reframe and become locality's showcase once it lands.
- **Tutorial home + generation mechanism:** MkDocs page under
  `docs-site/docs/examples/`; generator follows the `docs/demos/generate-docs.ts` →
  copy-into-`docs-site` precedent (there is **no** `pymdownx.snippets` include wiring
  today — would need adding to `mkdocs.yml:48` if we want file-includes vs paste).

---

## References

- Specs: `docs/specs/{keyed_models,batched_models,timeseries,maintenance_plan}.md`
- Emitters: `crates/smelt-logical/src/maintenance/emit.rs`
- Refusal guard: `crates/smelt-core/src/metadata.rs:537`
- Example base: `examples/web_analytics/` (+ `run_incremental.py`,
  `verify_incremental_equivalence.py`)
- User docs to fix: `docs-site/docs/{guide/materializations.md,reference/timeseries.md,reference/cumulative-aggregate.md}`
