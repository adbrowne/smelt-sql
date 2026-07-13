<!-- GENERATED FILE — edit examples/web_analytics/tutorial_pages/taking-stock.md and run python3 examples/web_analytics/generate_tutorial.py -->

# Taking stock

Four pages ago the pitch was: state the facts that make incremental
maintenance possible *in the SQL*, and let smelt derive the maintenance.
Here is the complete ledger for the pipeline you just built — every fact
you stated, and what smelt derived from it:

| You wrote (one clause each) | smelt derived |
|---|---|
| `WHERE event_date BETWEEN arrival − 3 days AND arrival` | Read windows widened 3 days back, everywhere: daily runs, every backfill chunk edge, propagation deltas. Plus the proof that a 3-day trailing re-run absorbs all late data. |
| `QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id …)` | A refusal — dedup across partitions isn't provably day-local — resolved by an explicit, documented override next to the code. |
| `RANGE BETWEEN INTERVAL '2 days' PRECEDING` frames in `sessionize` | The sessionizer's source lookback, without a config key. |
| `WHERE event_date BETWEEN session_start_date AND … + 1 day` | The inverted write window: a one-day run rewrites `[D−1, D+2)` of session partitions, so midnight-straddling sessions merge instead of splitting. |
| A self-reference in `sessions_chained` | Proof the table converges only when built oldest-first; sequential backfills enforced; excluded from propagation rather than mishandled. |
| One new `CASE … END AS is_purchase` column | A previewable `ALTER TABLE` migration; `NULL` history; full-rebuild fallback when in-place is impossible. |

Two properties sit under all of it, both pinned by tests in the repo, not
just asserted in prose: partition-at-a-time builds equal full rebuilds
(`per_partition_equivalence`), and every embedded SQL block in these pages
is regenerated from the real CLI (`tutorial_freshness`).

## What it buys

Scale the arithmetic from this example's generated feed (1M events over
60 days). A rebuild-the-world nightly job scans all 60 days to freshen
one; the derived plan reads at most 4 days of bronze for the same run —
and, unlike the nightly rebuild, that number stays flat as history grows.
The same shape holds per-change: a one-day late-data delta re-touched 1
day of parsed events, 4 of sessions, 6 of enrichment — not the tables.

The subtler purchase is trust. Every number in the execution — lookbacks,
rewrite spans, orderings — traces to a clause you can read, and the SQL
that runs is printable before it runs. Hand-built pipelines have those
numbers too; they're just unverified.

## What it costs — honestly

- **Your bounds must live in SQL the analyzer can read.** Literal
  `INTERVAL`s in filters and window frames — not parameters, not config
  the query ignores. Where SQL can't express a bound's *reason* (the
  dedup case), you carry an explicit override, and its correctness is on
  you, like any assertion.
- **Recompute, not update.** Maintenance is `DELETE`+`INSERT` of whole
  partitions. Idempotent and simple, but changing one row rewrites its
  partition; very large partitions argue for finer granularity.
- **Ordered models are second-class operationally** — sequential
  backfills, no propagation participation today. Reach for
  self-reference only when the semantics truly need memory.
- **Dataframe pipelines are more flexible, full stop.** If your transform
  is UDF-heavy, iterative, or reaches into ML libraries, Spark/pandas
  code does things smelt's SQL analysis cannot see; smelt's Python-model
  escape hatch runs such steps but derives nothing about them. smelt's
  bet is that the incremental-correctness machinery matters most exactly
  where the logic *is* expressible as SQL — which for warehouse
  analytics is most of it.
- **It's a young project.** Two backends (DuckDB, Spark); no automatic
  source watermarking yet; APIs still move. The [roadmap](https://github.com/adbrowne/smelt-sql/blob/main/docs/ROADMAP.md)
  is public.

## Where next

- The [incremental models guide](../../guide/incremental-models.md) — the
  reference treatment of everything these pages demonstrated, including
  the derivation rules ("Form A/B" bounds, refusal conditions, the
  derived output window).
- The [identity-stitching example](../web_analytics.md) — the gold layer
  of this same pipeline: three identity-merge strategies compared, and
  what happens when a computation is *global* rather than windowed.
- The [complete example project](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics)
  — everything from these pages plus tests, datagen config, and the
  equivalence-verification scripts.
