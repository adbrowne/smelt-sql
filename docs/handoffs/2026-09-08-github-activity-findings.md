# GitHub-activity pipeline findings — DuckDB half

**Status:** interim — **DuckDB half only**. The live-BigQuery half (compile refusals,
runtime failures, cross-target divergence) lands in phase 16 of
`docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`, gated on a provisioned GCP
project and credential that do not exist in this worktree (`gcloud auth list` → "No
credentialed accounts", re-checked through phase 9). Everything below is derived from
`examples/github_activity/`'s DuckDB replay over the committed 30-day Parquet fixture —
no live warehouse was queried to produce this document.

**Source of every claim below:** `docs/outcomes/20260906-bigquery-dogfood-spine/phases/`
`0{2,3,4,6,8,9}-summary.md`, that outcome's own "## Decision log", and
`examples/github_activity/README.md`. Nothing here is re-derived or re-measured; each
number is traceable to one of those.

## The four root causes

1. **Succession naming-tie fold** — `silver.repo_naming` and `silver.actor_naming`.
   The incremental window-forward patch loop addresses the presented table by
   `(key, clock)` (its `MERGE ... ON` clause), so a redelivered duplicate or a genuine
   same-second tie whose payload agrees converges to one presented row. `--full-refresh`
   re-runs the model's raw compiled `SELECT` (`emit_succession_full_rebuild`) with no such
   addressing, keeping every physically duplicated row. Measured exactly: 139 extra rows
   for `repo_naming`, 145 for `actor_naming`, matching the fixture's own same-second tie
   counts (`phases/03-summary.md`). `marts.naming_history` is unaffected — its `LAG` filter
   drops the duplicate identically on both legs. **Latent, unmeasured**: the rebuild path
   never runs the clock-tie probe at all, so a content-*disagreeing* tie (not just a
   byte-identical one) would silently corrupt a full refresh; this fixture measured zero
   disagreeing ties, so the gap stays undetected rather than exercised
   (`phases/03-summary.md`, `crates/smelt-cli/tests/github_activity_oracle.rs`'s
   `DIVERGENCE_REGISTRY` doc comment).

2. **Enrichment freeze** — `gold.events_enriched`. No `UpstreamMutation(gold.repo_dim)`
   maintenance cell is ever derived
   (`crates/smelt-logical/src/maintenance/derive/model_edge.rs::append_model_edge_cells`):
   the key-addressed route needs the *downstream's own* declared `unique_key`, which
   `gold.events_enriched` (`grain: partition`) has none of by construction, and the
   clock-based route needs the upstream to declare `timeseries:`, which `gold.repo_dim`
   (clockless) does not. Concretely, renaming a repo never refreshes
   `current_repo_name` on already-written rows through any tracked technique — silent,
   surfacing only via `smelt explain --json` (`RepairKeysNotDiscoverable`), never at `run`
   time. Measured over the full 30-day fixture (`every_window_deep_sweep`,
   `phases/08-summary.md`): the stale-row count is strictly non-decreasing across all 30
   days — `1, 1, 2, 5, 5, 8, 13, 15, 16, 22, 22, 22, 28, 28, 28, 29, 29, 29, 31, 36, 36,
   37, 37, 37, 37, 38, 38, 38, 39` — zero rows ever heal. This corrects an earlier, wrong
   phase 6 claim ("self-heals") that turned out to be a row-count check, not a content
   check.

3. **Oracle windowing gap** — `silver.actor_sessions`. `compute_calendar_windows`
   (`crates/smelt-runtime/src/windowing.rs`) applies the Form-B forward-reach rebase only
   at the two *outer* edges of a single multi-day invocation, never at an interior chunk
   boundary. A wide `--full-refresh` is exactly such an invocation, so the
   **full-refresh oracle itself under-counts** a cross-midnight session — the incremental
   leg is correct (confirmed against a from-scratch raw-SQL recomputation,
   `phases/08-summary.md`). Scope: any single invocation of a Form-B model spanning more
   than one partition chunk is affected, not just `--full-refresh` — a very wide ordinary
   incremental backfill window could show the same undercount.

4. **Mart repair gap** — `marts.daily_active_contributors`. This Form-A downstream
   aggregate has no rebase of its own and never revisits an already-written partition, so
   it never learns when `actor_sessions`'s own (correct) Form-B rebase rewrites an earlier
   partition. `total_events` is frozen at first-write time — a strict subset of the
   oracle. A third instance of the same missing-repair-edge shape as root cause 2's, but
   triggered by an ordinary self-rebase rather than a renamed dimension
   (`phases/08-summary.md`).

## The five registered divergences

| Relation | Bound | Root cause | Defect or fixture artifact |
|---|---|---|---|
| `silver_repo_naming` | `FoldEquality` (fold on `repo_id, created_at`) | 1 | Smelt defect (full-refresh rebuild path) |
| `silver_actor_naming` | `FoldEquality` (fold on `actor_id, created_at`) | 1 | Smelt defect (full-refresh rebuild path) |
| `gold_events_enriched` | `StaleButHistoricallyValid` (`current_repo_name` only; every stale value genuinely held earlier) | 2 | Smelt defect (missing maintenance cell) |
| `silver_actor_sessions` | `MonotoneDivergence` (oracle behind on `session_end`, `event_count`) | 3 | Smelt defect (oracle/windowing, not the incremental leg) |
| `marts_daily_active_contributors` | `MonotoneDivergence` (incremental behind on `total_events`) | 4 | Smelt defect (missing repair edge) |

(Full predicate parameters — key columns, exact-match columns — live in
`crates/smelt-cli/tests/github_activity_oracle.rs`'s `DIVERGENCE_REGISTRY`, not restated
here to avoid a second copy drifting from the registry.)

## Latent, unmeasured

`emit_succession_full_rebuild` never runs the clock-tie probe, so a content-*disagreeing*
tie (two rows sharing `(key, clock)` whose other columns differ) would corrupt a full
refresh undetected. This fixture measured zero disagreeing ties, so the gap is real but
unexercised — see root cause 1.

## Requirements handed to `20260906-external-dag-steps`

`scripts/bq-dogfood-loader.sh` is external to smelt by this outcome's own "Out of scope"
section: smelt's `raw.github_events` / `raw.github_events_arrival` source declarations are
the contract the loader is trusted against, but nothing in the smelt graph knows the
loader exists or runs on a schedule (`phases/09-summary.md`). What this costs today: a
reader of the smelt project cannot see, from smelt alone, that these two sources are
populated by an external scheduled query rather than by another smelt model or an
ungoverned manual load — the dependency is documented only in the source YAMLs'
`description:` prose and in this outcome's docs, not in anything smelt's graph, `explain`,
or lineage tooling can traverse. A `produced_by:` declaration on a source would need to
express, at minimum: (a) an external identifier for the producing job (the loader's own
derivation is a per-day `_TABLE_SUFFIX` slice plus a previous-day redelivery arm — see
`scripts/bq-dogfood-loader.sh::emit_sql`), so lineage tooling has something to point at;
(b) enough of the producer's own cadence (day-partitioned, one run per day) to let a
future staleness check compare "when did this source's data last land" against "when did
the declared producer last run" — which is exactly the axis this pipeline has no
column for today (see the `lateness: '6 hours'` placeholder in root cause discussion,
`phases/02-summary.md`); and (c) the redelivery arm's shape (`MOD(CAST(id AS BIGINT), 50)
= 0` over day D-1) as a *declared* fact rather than a comment the loader script and the
source YAML's `key_recurrence.window: '0 days'` must be kept in sync by hand.

## Requirements handed to `20260906-trimmed-history-sources`

The loader declares a **45-day** `partition_expiration_days` bound
(`scripts/bq-dogfood-loader.sh`, parsed from `README.md`'s "The BigQuery loader" section),
derived from the 30-day fixture range plus backfill headroom (`phases/09-summary.md`).
This is unrelated to and inconsistent with the pre-existing, inert
`retention: '90 days'` field on both `examples/github_activity/models/sources/raw/
github_events.yml` and `github_events_arrival.yml` — that field is parsed into
`smelt-core`'s `SourceDefinition` today but consumed by no maintenance logic (confirmed by
grep, no reader outside test fixtures). This outcome's phase 9 fixed the stale claim in
`github_events.yml`'s comment (it previously and incorrectly said the two numbers match)
but left the value itself alone, since owning the actual trimmed-history mechanism and
reconciling 45 vs. 90 belongs to `trimmed-history-sources`. That outcome must either: make
`retention:` a real, enforced bound and pick one number (with a stated reason for
diverging from the loader's 45, if it does), or otherwise formally connect the two so a
future reader cannot again find them silently disagreeing.

## Punch-list for `20260906-bigquery-correctness`

1. **Fix-or-register** the succession full-refresh rebuild path
   (`emit_succession_full_rebuild`) so it folds on `(key_cols, clock_col)` with an
   aggregate over every other column, closing the `silver_repo_naming` /
   `silver_actor_naming` divergence — provoked by `silver.repo_naming` and
   `silver.actor_naming`. (An exploratory `SELECT DISTINCT *` wrap was tried and found
   insufficient — see `phases/03-summary.md` — the real fix needs the full output schema
   threaded into the emitter.)
2. **Fix-or-register** the missing `UpstreamMutation(gold.repo_dim)` maintenance cell for a
   `grain: partition` downstream reading a clockless keyed-model dimension — provoked by
   `gold.events_enriched`. Needs either a new route in `append_model_edge_cells` for this
   combination, or a documented refusal surfaced at `run`/`build` time rather than only
   `explain`.
3. **Fix-or-register** `compute_calendar_windows`'s interior-chunk-boundary forward-reach
   loss for Form-B models — provoked by `silver.actor_sessions`, and by any other Form-B
   model materialized in one invocation spanning multiple partition chunks.
4. **Fix-or-register** the missing repair edge from a Form-B model's own self-rebase to a
   Form-A downstream aggregate that reads it verbatim — provoked by
   `marts.daily_active_contributors`. Worth checking whether the fix for item 2 (a
   general "downstream must learn an upstream rewrote an already-materialised row"
   mechanism) naturally covers this too, or whether they need separate maintenance-cell
   work.

## References

- `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md` — outcome header, criteria,
  and "## Decision log" (all dated 2026-09-08 unless noted).
- `docs/outcomes/20260906-bigquery-dogfood-spine/phases/0{2,3,4,6,8,9}-summary.md`.
- `crates/smelt-cli/tests/github_activity_oracle.rs` — `DIVERGENCE_REGISTRY` (the
  machine-checked source of truth for the five divergences' exact predicates).
- `examples/github_activity/README.md` — "Trusting the numbers", "The BigQuery loader".
