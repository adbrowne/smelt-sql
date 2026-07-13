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
design question is what the cut is anchored to, and this page builds the
answer that keeps partitions independent: anchor it to the clock. (The
other anchoring — the session's own start — is real too, has real uses,
and costs something surprising; it's at the end of this page.)

## The cut rule

`silver.sessions` uses the ordinary 30-minute gap rule (plus a platform
change starting a new session), with one added deadline: **a session dies
at the first midnight it fails to reach into.** Concretely, a session gets
to cross at most one midnight, and only if it has an event in the new
day's first 30 minutes — which a genuinely continuous session always does,
since its gaps are under 30 minutes. Follow that through and you get a
closed form worth stating, because everything else on this page leans on
it: **every session spans at most two calendar days.** The cutoff is
computable from a timestamp alone; no memory of the session's history is
needed.

## The model

<!-- smelt-include: models/silver/sessions.sql -->

Two things in here deserve names:

- **The sessionization is a reusable function.**
  `smelt.functions.sessionize`
  ([source](https://github.com/adbrowne/smelt-sql/blob/main/examples/web_analytics/functions/sessionize.sql))
  assigns each event its session's start timestamp using window functions,
  and its `RANGE BETWEEN INTERVAL '2 days' PRECEDING` frames are not just
  implementation — smelt reads them as the function's declared reach into
  the past. Functions expand transparently into the caller, so the
  planner analyzes the real SQL, not an opaque call. (More:
  [functions guide](../../guide/functions.md).)
- **The `WHERE` filter is a declaration, again.** `event_date BETWEEN
  session_start_date AND session_start_date + INTERVAL '1 day'` states
  the closed form from above in column terms: a session's events live on
  its start day or the day after, never further. The `HAVING` clause
  restates the same cap as a per-row assertion the emitted SQL enforces.
  And — same move as the lateness filter on the previous page — smelt
  derives windows from it. This time the derivation runs in the
  *opposite direction*, and that's the interesting part.

## The write window inverts the filter

For `events_parsed` on [the previous page](late-data.md), day D's output
depended on *earlier* source days, so
the **read** widened backward. A session table skews the other way: this
table is partitioned by `session_start_date`, and an event arriving on day
D can extend a session that *started on day D−1*. New data for day D can
change **yesterday's partition**.

smelt gets that from inverting the declared filter: if a session's events
reach at most one day past its start, then day D's events reach back to
sessions starting on D−1. So a run over `[D, D+1)` must rewrite
partitions `[D−1, D+2)` — and it does:

<!-- smelt-generate: @render=skeleton explain silver.sessions --show-sql --json --period 2026-04-10..2026-04-11 -->

??? example "Full emitted SQL — `smelt explain silver.sessions --show-sql --period 2026-04-10..2026-04-11`"

    <!-- smelt-generate: explain silver.sessions --show-sql --json --period 2026-04-10..2026-04-11 -->

Read the frame: the run window was one day, the `DELETE` covers
`session_start_date` in `[2026-04-09, 2026-04-11)`, and the events read
widened to cover both the session span and the sessionizer's two-day
lookback. Every bound traces to something declared in SQL you can point
at. (The `explain` output also shows a second cell — the same statements
triggered by new upstream data rather than an explicit backfill; the
[changing-things page](changing-things.md) uses that.)

## The payoff: a midnight-straddling session, handled by a one-day run

In the generated dataset there's a device with an event at
`2026-05-03 23:47` and its next at `2026-05-04 00:03` — a 16-minute gap,
one session, started on May 3rd. Now suppose you've already built
everything through May 3rd, and today's job runs May 4th:

<!-- smelt-generate: @render=skeleton explain silver.sessions --show-sql --json --period 2026-05-04..2026-05-05 -->

??? example "Full emitted SQL — `smelt explain silver.sessions --show-sql --period 2026-05-04..2026-05-05`"

    <!-- smelt-generate: explain silver.sessions --show-sql --json --period 2026-05-04..2026-05-05 -->

The May 4th run rewrites the **May 3rd** partition, folding the midnight
event into the existing session's row instead of minting a fragment
session at 00:03. This is the bug class — sessions split at partition
boundaries, session counts inflated — that hand-built day-at-a-time
session jobs get wrong by default, and that you otherwise fix by
remembering to over-rebuild ("always redo yesterday too") in a place far
from the session logic. Here the over-rebuild is derived, minimal, and
proven against the same filter the query enforces.

An end-to-end test in the repo
(`per_partition_equivalence.rs`) pins the stronger property all of this
is in service of: building this table day by day, in any order, produces
byte-identical results to building it from scratch in one pass.

## The alternative: let the session's own start decide

The clock-anchored deadline is a *design choice*, and a reasonable person
might prefer the other one: "a session ends roughly two days after it
started,"
measured from the session's own start. The full example builds that too,
as `silver.sessions_chained`
([source](https://github.com/adbrowne/smelt-sql/blob/main/examples/web_analytics/models/silver/sessions_chained.sql)),
with the same gap rule and attribution — the two tables differ only in
where the cap's timing comes from.

That one change transforms the execution. "When did the session I'm
continuing start?" cannot be answered from a bounded window of new events
— for a long-lived session, the start could be arbitrarily far back. The
model must consult **its own prior output**, and it does, via a
backward-bounded self-reference. smelt analyzes the self-reference and
proves a different property: the table still converges, but only if its
partitions are built **strictly in time order**. `explain` shows the
consequence — a third maintenance trigger, on the table itself:

<!-- smelt-generate: @render=skeleton explain silver.sessions_chained --show-sql --json --period 2026-04-10..2026-04-11 -->

??? example "Full emitted SQL — `smelt explain silver.sessions_chained --show-sql --period 2026-04-10..2026-04-11`"

    <!-- smelt-generate: explain silver.sessions_chained --show-sql --json --period 2026-04-10..2026-04-11 -->

What ordering costs, concretely: backfills of this table cannot be
parallelized (one partition at a time, oldest first), and — as the
[changing-things page](changing-things.md) shows — it opts the table out of automatic change propagation, which
refuses cycles. Nothing is wrong with paying that; the point is that
**smelt derived which table you built**, and tells you, instead of letting
a backfill quietly produce garbage in parallel.

The same repo tests pin how differently the three plausible designs treat
one pathological input — a device emitting an event every 29 minutes for
nine days straight, so the gap rule never fires and only the cap decides:

| Design | Result on the never-idle device | Execution |
|---|---|---|
| Clock-anchored cap (`silver.sessions`) | 9 sessions (~1/day) | partitions independent, parallel |
| Root-anchored cap (`silver.sessions_chained`) | 5 sessions (~1/2 days) | strictly ordered, sequential |
| Cap inside the window frame only (tempting; never shipped) | ~50 single-event sessions per day | "parallel," and wrong |

The third row is the cautionary one: a cap enforced only by a window
frame's reach *looks* partition-independent — no self-reference, nothing
for an analyzer to object to — but under the never-idle input the frame
simply stops containing what it needs, and session counts inflate 50×.
Session count is a headline metric. The difference between the first two
designs and the third is exactly the difference between a bound that is
*true of the data* and one that is merely present in the code.
