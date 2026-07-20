# Duplicates and late data

The feed has two hygiene problems the first model ignored — the
duplicates and the lateness from the overview, now with mechanisms.
Roughly one event in fifty is **redelivered**: a second copy arrives
later, identical except for its arrival time (at-least-once delivery
doing what it says). And events are **late**: in
this feed, a fifth of events arrive an hour or more after they
happened, and one in twenty arrives a full three days late.

Both fixes are one clause each. What's interesting is what smelt does
with those clauses — one it refuses until you justify it, the other it
turns into arithmetic you never have to maintain again.

## Deduplicating redeliveries — and a refusal

The natural dedup: keep the earliest-arriving copy of each `event_id`.

<!-- smelt-include: tutorial_stages/02_dedup_refused/models/silver/events_parsed.sql -->

Try to run it (`--dry-run` compiles and checks without executing):

```bash
smelt run --select silver.events_parsed \
  --event-time-start 2026-04-10 --event-time-end 2026-04-11 --dry-run
```

<!-- smelt-generate: @cwd=tutorial_stages/02_dedup_refused @render=text @expect-exit=1 run --select silver.events_parsed --event-time-start 2026-04-10 --event-time-end 2026-04-11 --dry-run -->

smelt refused the model, deliberately, and the reason repays attention.

Rebuilding a table one partition at a time is only correct if each
partition's contents can be computed without seeing the other partitions.
A window function is exactly the kind of thing that can break that: our
`ROW_NUMBER()` groups rows by `event_id`, and smelt cannot prove from the
SQL that all copies of one `event_id` land in the same `event_date`
partition. If they could straddle days, a day-by-day rebuild and a
full-history rebuild would disagree — silently, and only for the handful
of straddling events. That is precisely the class of bug that makes
hand-built incremental pipelines untrustworthy, and smelt would rather
stop than emit it.

Here, though, *we* know something the analyzer can't see: a redelivered
duplicate always carries the same `event_date` as its original — it's the
same event. That fact lives in the feed's contract, not in the SQL, so we
assert it where smelt can hold us to the location, in the model's
frontmatter:

```yaml
safety_overrides:
  allow_window_functions: true
```

(`safety_overrides:` is the top-level frontmatter key naming escape hatches
for the partition-grain safety checks that would otherwise refuse a pattern
like a window function over a redelivery-dedup window.)

The override is a signed statement, sitting next to the window function
it excuses, with a comment explaining why it's safe (see the stage-3
model below). Compare the alternatives you may have lived with: dbt will
happily template whatever dedup you write into an incremental model;
nothing asks whether it is partition-safe, so the question surfaces
later, as a data bug.

## Accepting late arrivals

Late data needs a policy, not just plumbing. Ours: an event may arrive up
to three days after it happened; anything later is dropped as too old to
resurrect. In SQL, that policy is one filter comparing the two clocks —
here in the finished version of the model (dedup override included):

<!-- smelt-include: tutorial_stages/03_late_data/models/silver/events_parsed.sql -->

The `WHERE` clause reads as documentation: *accept an event if its
occurrence day is within 3 days behind its arrival day.* Every event in
the table provably satisfies it, however the table gets built.

## The derived lookback

That same filter is load-bearing for the planner. Because it ties the
model's partition column (`event_date`) to another clock (`arrival_time`)
by a bounded interval, smelt derives: **to rebuild day D, reading
`[D − 3 days, D]` of the source is sufficient.** Look at what the same
one-day run compiles to now:

<!-- smelt-generate: @cwd=tutorial_stages/03_late_data explain silver.events_parsed --show-sql --json --period 2026-04-10..2026-04-11 -->

The write window is unchanged — `[2026-04-10, 2026-04-11)` — but the read
of `bronze_raw_events` widened to `[2026-04-07, 2026-04-11)`: the
three-day acceptance window, derived from the filter, not configured
anywhere. The read being wider than the write is also why a new wrapper
appears: `_smelt_output_clamp` filters the result back down to exactly
the write window before the `INSERT`, so the extra source days inform
the computation without leaking extra rows into the table. Change the `INTERVAL '3 days'` to `'7 days'` and every window
smelt ever emits for this model follows; there is no second copy of the
number to forget.

The derivation also answers the operational question every late-data
pipeline has to answer — *how far back must my daily job look?* Because
the filter guarantees nothing older than three days can enter the table,
re-running a trailing window that covers those days is **provably
sufficient**, not hopefully sufficient:

```bash
# Each day, refresh today plus the three days late data can still reach
smelt run --event-time-start 2026-04-08 --event-time-end 2026-04-12
```

This is the move the rest of smelt's documentation calls **deriving
properties**: an operational fact — here, how far back a rebuild must
read — is read out of the SQL and proven, instead of being declared
somewhere beside it and trusted.

(The [changing-things page](changing-things.md) shows the surgical alternative — telling smelt exactly which
upstream days received data and letting it rebuild only what's affected.)

If you've built this in other tools, the contrast is the point:

- **dbt**: the lookback is a number in a macro —
  `WHERE event_date >= dateadd(day, -3, ...)` inside `is_incremental()` —
  with no stated relationship to any acceptance rule. When the two drift,
  nothing fails; numbers just go quietly wrong.
- **SQLMesh**: closer — `lookback` is a first-class model config key. But
  it is still a declared number the engine trusts, not a fact derived
  from the query; the SQL and the config can disagree.
- **Spark**: the reprocess window lives in job code or, in Structured
  Streaming, as a `withWatermark` threshold — either way a configured
  number one layer away from the logic it must agree with. The
  `INTERVAL '3 days'` here plays the watermark's role, read out of the
  query instead of set beside it.

smelt's version is one clause that is simultaneously the policy, its
enforcement, and the source the windows are derived from.

The dedup override above is a debt this series doesn't have to carry: the
[next page](deduplication.md) builds the same dedup as a declared,
checked fact instead of a trusted comment.
