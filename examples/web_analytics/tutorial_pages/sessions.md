# Sessions and the cross-midnight backfill

Sessions are where "rebuild a day at a time" earns its complications. A
session — a run of one device's events with no 30-minute gap — is defined
by *relationships between rows*, and those relationships don't respect
your partitions:

- A session that starts at 23:47 and keeps going belongs to one day's
  partition but is built from two days' events.
- Worse: nothing in the definition stops a session from going on forever.
  One kiosk display, background sync, or misbehaving client that never
  pauses for 30 minutes produces a session with no end — a row that is
  never final, and that only a full-history scan can rebuild.

So **any sessionizer that you want to maintain incrementally has to cut
long sessions somewhere.** That's not a smelt rule; it's arithmetic. The
design question is what the cut is anchored to. This page builds the
answer that keeps partitions independent — anchor it to the clock — and
closes with the other answer, which costs more than it looks like it
should.

## The cut rule

`silver.sessions` keeps the ordinary 30-minute gap rule (plus a platform
change starting a new session) and adds one deadline on top. The gap
rule ends a session when the user pauses; the deadline exists so a
session that never pauses still ends: a session dies at the first
midnight it fails to reach into, where "reaching into" a day means
having an event in its first 30 minutes. A session that genuinely
crosses a midnight always reaches into the new day (its gaps are under
30 minutes, so some event lands within 30 minutes of the boundary),
which means the deadline only ever fires at the *next* midnight after
that. Two consequences, and everything on this page leans on them:
a session can cross at most one midnight, so **every session spans at
most two calendar days** — and whether it must end is computable from a
timestamp alone, with no memory of the session's history.

## The model

<!-- smelt-include: models/silver/sessions.sql -->

Three things in here deserve names:

- **The sessionization is a reusable function.**
  `smelt.functions.sessionize`
  ([source](https://github.com/adbrowne/smelt-sql/blob/main/examples/web_analytics/functions/sessionize.sql))
  assigns each event its session's start timestamp using window functions.
  (The `source =>` syntax is smelt's named-argument form for function
  calls — not standard SQL.) Its `RANGE BETWEEN INTERVAL '2 days'
  PRECEDING` frames are not just implementation: smelt reads them as the
  function's declared reach into the past. Functions expand transparently
  into the caller, so the planner analyzes the real SQL, not an opaque
  call. (More: [functions guide](../../guide/functions.md).)
- **The `WHERE` filter is another declaration.** `event_date BETWEEN
  session_start_date AND session_start_date + INTERVAL '1 day'` states
  the two-calendar-day rule in column terms: a session's events live on
  its start day or the day after, never further. The `HAVING` clause
  restates the same cap as a per-row assertion the emitted SQL enforces.
  And, same move as the lateness filter on the previous page, smelt
  derives windows from it — this time in the opposite direction.
- **The attribution expression is just SQL.** `ARG_MAX(utm_campaign,
  -epoch_us(event_ts)) FILTER (WHERE …)` picks the earliest non-NULL
  campaign within the session's first five minutes — negating the
  timestamp turns "value at the maximum" into "value at the earliest
  event." It plays no role in the maintenance derivation; it's here so
  the pipeline computes something a marketer would recognize.

## The write window inverts the filter

For `events_parsed` on [the previous page](late-data.md), day D's output
depended on *earlier* source days, so the **read** widened backward. A
session table skews the other way: this table is partitioned by
`session_start_date`, and an event arriving on day D can extend a session
that *started on day D−1*. New data for day D can change **yesterday's
partition**.

smelt gets that by inverting the declared filter: if a session's events
reach at most one day past its start, then day D's events reach back to
sessions starting on D−1. A run over `[D, D+1)` must therefore rewrite
partitions `[D−1, D+1)` — yesterday's and today's — and it does:

<!-- smelt-generate: @render=skeleton explain silver.sessions --show-sql --json --period 2026-04-10..2026-04-11 -->

??? example "Full emitted SQL — `smelt explain silver.sessions --show-sql --period 2026-04-10..2026-04-11`"

    <!-- smelt-generate: explain silver.sessions --show-sql --json --period 2026-04-10..2026-04-11 -->

Read the frame: the run window was one day, the `DELETE` covers
`session_start_date` in `[2026-04-09, 2026-04-11)`, and the events read
widened to cover both the session span and the sessionizer's two-day
lookback. Every bound traces to something declared in SQL you can point
at. (The output also lists a second statement group under a `NewData`
trigger — the same work, run when upstream data changes rather than when
you ask for a window; the [changing-things page](changing-things.md)
puts that to use.)

## The payoff: a midnight-straddling session, handled by a one-day run

In the generated dataset there's a device with an event at
`2026-05-03 23:47` and its next at `2026-05-04 00:03` — a 16-minute gap,
one session, started on May 3rd. Suppose you've already built everything
through May 3rd, and today's job runs May 4th:

<!-- smelt-generate: @render=skeleton explain silver.sessions --show-sql --json --period 2026-05-04..2026-05-05 -->

The May 4th run rewrites the **May 3rd** partition, folding the midnight
event into the existing session's row instead of minting a fragment
session at 00:03. This is the bug class — sessions split at partition
boundaries, session counts inflated — that hand-built day-at-a-time
session jobs get wrong by default, and that you otherwise fix by
remembering to over-rebuild ("always redo yesterday too") in a place far
from the session logic. Here the over-rebuild is derived, minimal, and
proven against the same filter the query enforces.

An end-to-end test in the repo (`per_partition_equivalence.rs`) pins the
property all of this serves: building this table day by day, in any
order, produces results identical to building it from scratch in one
pass.

## A different cut, a different execution shape

The clock-anchored deadline is a design choice, and a reasonable person
might prefer the other one: "a session ends roughly two days after it
*started*," measured from the session's own start. The full example
builds that too, as `silver.sessions_chained`, with the same gap rule and
attribution; the two tables differ only in where the cap's timing comes
from.

That one change transforms the execution. "When did the session I'm
continuing start?" cannot be answered from any bounded window of new
events — for a long-lived session, the start could be arbitrarily far
back. The model must consult **its own prior output**, and smelt, seeing
the self-reference, proves a different property: the table still
converges, but only if its partitions are built strictly in time order.
smelt enforces that ordering itself (backfills run oldest-first, one
partition at a time, never in parallel), and — as the
[changing-things page](changing-things.md) shows — the self-reference
also opts the table out of automatic change propagation. The
[deep dive](ordered-sessions.md) walks the model and its emitted plan.

What makes the choice worth a page of its own is how differently the
three plausible designs treat one pathological input — a device emitting
an event every 29 minutes for nine days straight, so the gap rule never
fires and only the cap decides:

| Design | Result on the never-idle device | Execution |
|---|---|---|
| Clock-anchored cap (`silver.sessions`) | 9 sessions (~1/day) | partitions independent |
| Root-anchored cap (`silver.sessions_chained`) | 5 sessions (~1/2 days) | strictly ordered, sequential |
| Cap inside the window frame only (this example's original design, since replaced) | ~50 single-event sessions per day | "independent," and wrong |

(If you build streaming pipelines: the clock-anchored table is roughly
what `session_window` can express; the root-anchored one is loosely the
shape you'd otherwise reach for stateful processing for.)

The third row is the cautionary one. A cap enforced only by a window
frame's reach *looks* partition-independent — no self-reference, nothing
for an analyzer to object to — but under the never-idle input the frame
simply stops containing what it needs, and session counts inflate 50×.
Session count is a headline metric. The difference between the first two
designs and the third is exactly the difference between a bound that is
*true of the data* and one that is merely present in the code.
