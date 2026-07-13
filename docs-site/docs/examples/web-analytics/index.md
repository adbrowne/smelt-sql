<!-- GENERATED FILE — edit examples/web_analytics/tutorial_pages/index.md and run python3 examples/web_analytics/generate_tutorial.py -->

# Building an incremental web-analytics pipeline

This is a worked example, start to finish, of building event analytics on
smelt: an event feed with duplicates and multi-day late arrivals, sessions
that cross midnight, and first-touch campaign attribution. It is written
for someone who has never used smelt. You need SQL; you do not need dbt,
SQLMesh, or Spark experience — where a step replaces something you would
do in those tools, the text says so.

## The problem

You run product analytics. Events stream in from web and mobile clients,
and you want daily tables — parsed events, sessions, attributed campaigns —
that stay correct as data keeps arriving. Three facts about the feed make
"stay correct" harder than it sounds:

- **Duplicates.** Delivery is at-least-once: about 2% of events arrive
  twice, identical except for their arrival time.
- **Late data.** An event that happened on Monday can arrive on Thursday —
  ingestion trails occurrence by up to three days.
- **Sessions.** Your headline metrics are defined over sessions (runs of
  activity separated by 30-minute gaps), and a session that starts at
  23:47 doesn't care that your tables are organized in day-sized slices.

Teams usually build this one of two ways:

1. **Rebuild everything, every night.** Correct by construction and easy
   to reason about, but the cost grows with history: a year in, you are
   rescanning twelve months of events to update one day.
2. **Hand-built incremental jobs.** `MERGE` statements, or jobs that
   re-replace a trailing slice of the table ("reprocess the last 3
   days"), with the window widths
   encoded in orchestration config, Jinja macros, or job code. Fast and
   cheap, but every one of those numbers is a promise someone made once.
   Change the session rule and the 3 stops being right, and nothing tells
   you.

smelt's position is that the numbers should not be promises. You write the
plain, full-history query, and where incremental maintenance depends on a
fact about the data — how late events can be, how long a session can span —
you state that fact **in the SQL itself**, as an ordinary filter or window
frame. smelt reads the query, derives the maintenance plan from it (what to
read, what to rewrite, in what order), and shows you the exact SQL it will
run before it runs it. When it cannot prove a model is safe to maintain
incrementally, it refuses and says why, rather than guessing.

Every SQL block in these pages is real CLI output, captured by a
[generator script](https://github.com/adbrowne/smelt-sql/blob/main/examples/web_analytics/generate_tutorial.py)
and re-verified by a CI test — the pages cannot drift from what smelt
actually emits.

## What we build

```text
raw.events  (source: at-least-once feed, up to 3 days late)
    │
bronze.raw_events        typed passthrough
    │
silver.events_parsed     dedup + late-arrival window
    │
silver.sessions          30-min sessionization
    │
silver.events_enriched   session identity joined back
```

(The `bronze`/`silver` names are this example's layering convention —
raw, then cleaned — not smelt keywords.)

The [complete example](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics)
also carries a gold identity-resolution layer, covered separately in
[the identity-stitching example](../web_analytics.md). These pages build
the pipeline up in stages — each stage is a small, runnable smelt project
under
[`tutorial_stages/`](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics/tutorial_stages),
so you can run every command yourself.

## The pages

1. **[A first incremental model](first-model.md)** — partitions, run
   windows, and reading the exact SQL a run will execute.
2. **[Duplicates and late data](late-data.md)** — a deduplication that
   smelt refuses until you justify it, and a lateness rule smelt turns
   into derived read windows.
3. **[Sessions and the cross-midnight backfill](sessions.md)** — why any
   incrementally-maintained sessionizer must cut somewhere, and how a run
   for today correctly rewrites yesterday's partition. (An optional
   [deep dive](ordered-sessions.md) covers the session table that has to
   read its own history.)
4. **[Backfills, new columns, and late updates](changing-things.md)** —
   the three ways a live pipeline changes, and what smelt does for each.
5. **[Taking stock](taking-stock.md)** — what you wrote versus what smelt
   derived, and an honest comparison with the alternatives.

## Run it locally

```bash
pip install smelt-sql
git clone https://github.com/adbrowne/smelt-sql.git
cd smelt-sql/examples/web_analytics

# Generate the synthetic feed (1M events at scale 1.0; 0.01 is plenty)
smelt-datagen --config datagen.yaml --scale-factor 0.01
duckdb target/dev.duckdb < setup_sources.sql

smelt build
```

Every `smelt explain` and `--dry-run` command in these pages works with
no data at all — they print what *would* run. To execute real runs inside
a stage project, copy `datagen.yaml` and `setup_sources.sql` into the
stage directory and repeat the two data steps there.
