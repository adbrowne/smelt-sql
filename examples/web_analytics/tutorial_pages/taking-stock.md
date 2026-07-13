# Taking stock

Five pages ago the pitch was: state the facts that make incremental
maintenance possible *in the SQL*, and let smelt derive the maintenance.
Here is the complete ledger for the pipeline you just built — every fact
you stated, and what smelt derived from it:

| You wrote (one clause each) | smelt derived |
|---|---|
| `WHERE event_date BETWEEN arrival − 3 days AND arrival` | Read windows widened 3 days back, everywhere: daily runs, every backfill chunk edge, propagation deltas. Plus the proof that a 3-day trailing re-run absorbs all late data. |
| `QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id …)` | A refusal — dedup across partitions isn't provably day-local — resolved by an explicit, documented override next to the code. |
| `RANGE BETWEEN INTERVAL '2 days' PRECEDING` frames in `sessionize` | The sessionizer's source lookback, without a config key. |
| `WHERE event_date BETWEEN session_start_date AND … + 1 day` | The inverted write window: a one-day run also rewrites yesterday's session partition (`[D−1, D+1)`), so midnight-straddling sessions merge instead of splitting. |
| A self-reference in `sessions_chained` | Proof the table converges only when built oldest-first; sequential execution enforced; excluded from propagation rather than mishandled. |
| One new `CASE … END AS is_purchase` column | A previewable `ALTER TABLE` migration; `NULL` history; full-rebuild fallback when in-place is impossible. |

Two properties sit under all of it, both pinned by tests in the repo, not
just asserted in prose: partition-at-a-time builds equal full rebuilds
(`per_partition_equivalence`), and every embedded SQL block in these pages
is regenerated from the real CLI (`tutorial_freshness`).

## What it buys

On this example's generated feed (1M events over 60 days):

| Situation | Rebuild-the-world job | This pipeline |
|---|---|---|
| Daily freshen, day 60 | scans all 60 days, and more every day | reads ≤ 4 days of bronze; flat as history grows |
| One day of late data lands | rerun everything downstream | 1 day of parsed events + 4 of sessions + 6 of enrichment, derived per edge |
| 18-day backfill | one monolithic rebuild | bounded chunks, each with its own derived edge-widening |

Hand-built incremental pipelines reach the same run costs; the difference
is that their window numbers are constants someone chose, and verifying
them is separate machinery (tests, freshness checks) living apart from
the queries it guards. Here the numbers are derived from the queries, or
the model is refused.

## What it costs — honestly

- **Your bounds must live in SQL the analyzer can read.** Literal
  `INTERVAL`s in filters and window frames — not parameters, not config
  the query ignores. Where SQL can't express a bound's *reason* (the
  dedup case), you carry an explicit override, and its correctness is on
  you, like any assertion.
- **Recompute, not update.** Partition-grain maintenance is
  `DELETE`+`INSERT` of whole partitions: idempotent and simple, but
  changing one row rewrites its partition. (The other maintenance shape,
  `grain: key`, keeps one merged row per key with derived combiners —
  closer to a `MERGE` — see
  [key-grain patterns](../../reference/cumulative-aggregate.md). This
  tutorial's models are all partition-grain.)
- **Batch, not streaming.** Runs process explicit time windows; there is
  no continuous mode.
- **Ordered models are second-class operationally** — sequential
  backfills, no propagation participation today. Reach for
  self-reference only when the semantics truly need memory
  ([deep dive](ordered-sessions.md)).
- **Dataframe pipelines are more flexible, full stop.** UDF-heavy,
  iterative, or ML-library transforms do things smelt's SQL analysis
  cannot see, and smelt has no runtime for them —
  [Python models](../../guide/python-models.md) exist, but they are
  compile-time SQL generators, not an escape into pandas. smelt's bet is
  that the correctness machinery matters most exactly where the logic
  *is* expressible as SQL, which for warehouse analytics is most of it.
- **It's a young project.** Two backends — DuckDB and
  [Spark](../../guide/targets.md), where models materialize as Delta
  tables by default; no automatic source watermarking yet; APIs still
  move. The
  [roadmap](https://github.com/adbrowne/smelt-sql/blob/main/docs/ROADMAP.md)
  is public.

## Operating it

smelt is a CLI, not a scheduler. Run state — processed-interval history,
run manifests, deployed-schema baselines — lives in a `.smelt/` directory
beside the project, next to the warehouse tables themselves; scheduling
is whatever you already use (cron, an orchestrator) invoking
`smelt run --auto` or explicit windows. Data tests ship with the example
(`smelt test`); the [CLI reference](../../reference/cli.md) has the full
command surface.

## Where next

- The [incremental models guide](../../guide/incremental-models.md) — the
  reference treatment of everything these pages demonstrated: the exact
  filter shapes the analyzer recognizes (it names them Form A and
  Form B), when a model is refused, and the derived output window.
- The [identity-stitching example](../web_analytics.md) — the gold layer
  of this same pipeline: three identity-merge strategies compared, and
  what happens when a computation is *global* rather than windowed.
- The [complete example project](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics)
  — everything from these pages plus tests, datagen config, and the
  equivalence-verification scripts.

The subtler thing this pipeline bought is trust: every number in its
execution — lookbacks, rewrite spans, orderings — traces to a clause you
can read, next to the logic it protects, and smelt would rather refuse a
model than guess about one it can't prove.
