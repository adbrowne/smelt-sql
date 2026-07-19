<!-- GENERATED FILE — edit examples/web_analytics/tutorial_pages/ordered-sessions.md and run python3 examples/web_analytics/generate_tutorial.py -->

# Deep dive: the session table that reads itself

*Optional. This page expands the closing section of
[Sessions and the cross-midnight backfill](sessions.md): the root-anchored
sessionizer, why it must read its own history, and what that costs
operationally. Skip it on a first pass; nothing later depends on it.*

## The rule, and why bounded reads can't implement it

`silver.sessions_chained` keeps the 30-minute gap rule and the 5-minute
attribution window of `silver.sessions`, but replaces the midnight
deadline with a root-anchored one: a new event continues an open session
only if that session started less than two calendar days ago; otherwise
it starts a new one.

The clock-anchored table never needs to remember anything: its deadline
is computable from any event's own timestamp. The root-anchored deadline
is not. To decide whether a day-D event continues a session, you must
know *when the open session started* — and for a device that has been
active continuously, that start recedes arbitrarily far into the past.
No fixed-width read of recent events can recover it. The information
lives in exactly one bounded place: the session table's own previous
output.

## The self-reference

So the model reads it. For each candidate chain of new events, the SQL
looks up — in `silver.sessions_chained` itself — the most recent session
it already wrote for that device within the previous two days, and
continues it if the gap from that session's last event is under 30
minutes ([model source](https://github.com/adbrowne/smelt-sql/blob/main/examples/web_analytics/models/silver/sessions_chained.sql);
the lookup is a set of correlated subqueries against the model's own
table, bounded to a two-day reach backward and none forward).

That bounded, backward-only self-reference is what smelt's analyzer keys
on. A table that consumes its own output converges to the same answer as
a from-scratch build only if partitions are produced in time order — each
partition must be able to trust that everything before it is final. smelt
proves the self-read is backward-bounded, classifies the model as
**ordered**, and enforces the consequence itself: partitions build
oldest-first, one at a time, in every run and backfill. You run the same
commands as for any other model; the ordering is not your job.

`explain` makes the classification visible. Alongside the `Backfill` and
upstream `NewData` triggers every incremental model has, this table gets
a third: a `NewData` trigger on *itself* — the self-edge, surfaced as
part of the plan:

<!-- smelt-generate: @render=skeleton explain silver.sessions_chained --show-sql --json --period 2026-04-10..2026-04-11 -->
```sql
-- trigger: Backfill
BEGIN
  DELETE FROM main.silver_sessions_chained WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
  INSERT INTO main.silver_sessions_chained SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
COMMIT

-- trigger: NewData { source: "silver.events_deduped" }
BEGIN
  DELETE FROM main.silver_sessions_chained WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
  INSERT INTO main.silver_sessions_chained SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
COMMIT

-- trigger: NewData { source: "silver.sessions_chained" }
BEGIN
  DELETE FROM main.silver_sessions_chained WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
  INSERT INTO main.silver_sessions_chained SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
COMMIT
```

## What ordering costs

- **Backfills serialize.** An 18-month rebuild of the clock-anchored
  table can proceed in any order, chunk by chunk. The same rebuild here
  runs strictly oldest-first, one partition per step.
- **No automatic change propagation.** `smelt run --since-upstream`
  ([changing-things page](changing-things.md)) refuses graphs containing
  self-referential nodes rather than guessing at them; a pipeline that
  includes this table falls back to explicit window re-runs.
- **First-run seeding matters.** The earliest partition has no prior
  output to consult; the model's bounded lookback defines what "no open
  session" means there. (See the
  [incremental models guide](../../guide/incremental-models.md#self-referential-ordered-models)
  for the general rules.)

If you're mapping this to other stacks: this is loosely the batch analog
of stateful stream processing — with the difference that the "state" is
the table's own committed output rather than a mutable state store, and
the checkpoint discipline is the derived ordering constraint.

None of this is a defect — the root-anchored semantics genuinely require
memory, and smelt's contribution is proving *which* execution discipline
makes those semantics converge, then enforcing it, instead of letting a
parallel backfill quietly corrupt the table. But the costs are real,
which is why the main page's advice stands: order only what needs
ordering.
