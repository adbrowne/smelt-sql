# Backfills, new columns, and late updates

A pipeline that only ever moves forward one day at a time is the easy
case. Real ones change in three distinct ways, and it's worth keeping
them distinct, because the correct response to each is different:

1. **Backfill** — recompute history that already has a definition: a new
   model needs the past year, or a bug fix invalidates March.
2. **A new column** — the *shape* changes: the query grows a column that
   history never had.
3. **Late updates** — the definition and shape are fine, but upstream
   data changed: late arrivals landed, a source was corrected.

This page runs each one against the pipeline as built so far, on the
stage projects ([stage 3](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics/tutorial_stages/03_late_data),
[stage 4](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics/tutorial_stages/04_add_column),
[stage 5](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics/tutorial_stages/05_enrichment)).

## Backfilling a range

`smelt backbuild` rebuilds a model (and anything upstream it needs) over
a date range. `--dry-run` prints the full plan without executing:

```bash
smelt backbuild silver.events_parsed --start 2026-04-01 --end 2026-04-19 --dry-run
```

<!-- smelt-generate: @cwd=tutorial_stages/03_late_data @render=skeleton backbuild silver.events_parsed --start 2026-04-01 --end 2026-04-19 --dry-run -->

??? example "Full dry-run transcript"

    <!-- smelt-generate: @cwd=tutorial_stages/03_late_data backbuild silver.events_parsed --start 2026-04-01 --end 2026-04-19 --dry-run -->

The 18-day range didn't become one giant statement: it was split into
chunks (`-- chunk 1/2`, `-- chunk 2/2`), each its own transactional
`DELETE`+`INSERT`, so a long backfill holds bounded memory and commits
incrementally. Notice each chunk's *read* carries its own three-day
widening at the edge — `chunk 1` writes `[04-01, 04-10)` but reads bronze
from `03-29` — the late-arrival derivation from
[the late-data page](late-data.md), composing with chunking instead of
being re-derived by hand for the backfill case.

Because this model's partitions are independent, no ordering between
chunks is required for correctness. For the ordered sessionizer in
[the deep dive](ordered-sessions.md), the same command enforces
oldest-first instead — same interface, different derived discipline.

## Adding a column

Add a derived flag to `events_parsed` — one new line in the SELECT:

```sql
    CASE WHEN json_extract_string(payload, '$.event_name') = 'purchase'
         THEN TRUE
    END AS is_purchase
```

History was built without this column, so model and table now disagree.
`smelt diff` compares the model against the deployed schema and shows the
migration it implies, before anything runs:

```bash
smelt diff --select silver.events_parsed
```

<!-- smelt-generate: @cwd=tutorial_stages/04_add_column @fixture-schemas @render=text @expect-exit=1 diff --select silver.events_parsed -->

The change classifies as **safe**: adding a nullable column needs no
rewrite. The next `smelt run` applies the `ALTER TABLE` automatically and
carries on incrementally — already-built partitions keep `NULL` for
`is_purchase`, and partitions built from now on populate it. (This exact
flow — baseline saved, column added, `ALTER` applied, old rows `NULL`,
new rows populated — is pinned end-to-end by
`e2e::schema_evolution_incremental` in the repo.)

Two follow-ups you'll want eventually:

- **Populating history.** `NULL` history is often fine (the flag simply
  starts existing). When it isn't, declare a `backfill:` expression for
  the column in frontmatter, or `smelt backbuild` the range — the
  definition already computes the column, so a backfill fills it. See
  [schema evolution](../../guide/schema-evolution.md).
- **The not-nullable trap.** If the new column is provably `NOT NULL`
  (say, `amount >= 100` over a non-null `amount`), an in-place `ALTER`
  is impossible — `NULL` history would violate the type — and smelt
  responds with a **full-table rebuild** on the next run. Correct, but
  on a large table you want to choose the moment. Keep intentionally
  nullable additions nullable (our `CASE` has no `ELSE` for exactly this
  reason), or declare a `default:`.

For comparison: dbt's `on_schema_change: append_new_columns` does the
`ALTER` but nothing checks nullability against history, and sync is
opt-in per model. SQLMesh's plan preview is the nearest analog to
`smelt diff` — it categorizes changes as breaking or non-breaking before
applying. Spark's `mergeSchema` appends columns on write. The difference
here is mostly that the classification extends to the physical migration
statement, shown before the run, with the unsafe variant refused rather
than attempted.

## When upstream data changes

Late arrivals were accepted on [the late-data page](late-data.md) by
*policy*; this is the mechanics of absorbing them. Suppose the feed
delivers a batch of stragglers whose `event_date` is April 10th —
bronze's April 10th partition just gained rows, today. Everything
computed *from* that day is now stale: parsed events for the 10th, any
session that day's events participate in, and every enriched row that
referenced those sessions.

You could re-run a generous window over the whole pipeline. The precise
alternative: tell smelt what landed, and it answers with a **dirty set**
— every model-and-window pair the declared delta could have made stale —
then runs exactly that, in dependency order:

```bash
smelt run --since-upstream \
  --source silver.events_parsed --landed 2026-04-10..2026-04-11 --dry-run
```

<!-- smelt-generate: @cwd=tutorial_stages/05_enrichment @render=dirty-set run --since-upstream --source silver.events_parsed --landed 2026-04-10..2026-04-11 --dry-run -->

The widening rule is the same one you've been reading all along, applied
in reverse: **a partition is dirty if its read window touches the
delta.** A `sessions` partition P reads events from two days before P
(the sessionizer's lookback) through one day after (the session span) —
so a one-day delta dirties the four session partitions `[04-09, 04-13)`.
An `events_enriched` partition reads sessions from a day either side of
itself, so those four dirty session days spread to six enriched days,
`[04-08, 04-14)`. None of it is configured in the run command; every
number is the inversion of a filter you've already seen declared.

The stage-5 project this runs against contains the enrichment model —
event rows with their session identity joined back on
([source](https://github.com/adbrowne/smelt-sql/blob/main/examples/web_analytics/tutorial_stages/05_enrichment/models/silver/events_enriched.sql)) —
but not the ordered `sessions_chained` table. That's the cost from
[the sessions page](sessions.md) arriving on schedule: propagation
refuses graphs with self-referential nodes rather than guessing at them,
so the full example (which keeps both session tables) currently answers
`--since-upstream` with a named refusal, and the workaround is the
generous-window re-run.

!!! note "Two pragmatic notes"
    `--landed` windows come from you or your loader's bookkeeping —
    smelt does not yet watermark-diff sources automatically. And for the
    simple "resume where I left off" case, `smelt run --auto` fills
    whatever intervals haven't been processed yet
    ([CLI reference](../../reference/cli.md)).

The through-line of all three changes is the same: you described *what
changed* — a range, a column, a landed window — and every bound in
smelt's response was derived from declarations the pipeline had already
made.
