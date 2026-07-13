<!-- GENERATED FILE — edit examples/web_analytics/tutorial_pages/sessions.md and run python3 examples/web_analytics/generate_tutorial.py -->

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
```sql
---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: session_start_date
  partition_column: session_start_date
  granularity: day
---
WITH sessionized AS (
    SELECT
        device_id,
        event_ts,
        event_date,
        platform,
        utm_campaign,
        session_start_ts,
        CAST(session_start_ts AS DATE) AS session_start_date
    FROM smelt.functions.sessionize(
        source => smelt.silver.events_parsed,
        partition_col => device_id,
        ts_col => event_ts,
        platform_col => platform
    )
)
SELECT
    CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
    device_id,
    session_start_ts,
    session_start_date,
    MIN(event_ts) AS session_start,
    MAX(event_ts) AS session_end,
    COUNT(*) AS event_count,
    ANY_VALUE(platform) AS platform,
    ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
        WHERE utm_campaign IS NOT NULL
            AND event_ts <= session_start_ts + INTERVAL '5 minutes'
    ) AS utm_campaign
FROM sessionized
WHERE event_date
    BETWEEN session_start_date
        AND session_start_date + INTERVAL '1 day'
GROUP BY device_id, session_start_ts, session_start_date
HAVING MAX(event_ts) - MIN(event_ts) < INTERVAL '2 days' -- max_session_span: explicit, checkable cap assertion
```

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
```sql
-- trigger: Backfill
BEGIN
  DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
  INSERT INTO main.silver_sessions SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
COMMIT

-- trigger: NewData { source: "silver.events_parsed" }
BEGIN
  DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
  INSERT INTO main.silver_sessions SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
COMMIT
```

??? example "Full emitted SQL — `smelt explain silver.sessions --show-sql --period 2026-04-10..2026-04-11`"

    <!-- smelt-generate: explain silver.sessions --show-sql --json --period 2026-04-10..2026-04-11 -->
    ```sql
    -- trigger: Backfill
    BEGIN
      DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
      INSERT INTO main.silver_sessions SELECT * FROM (
      WITH sessionized AS (
          SELECT
              device_id,
              event_ts,
              event_date,
              platform,
              utm_campaign,
              session_start_ts,
              CAST(session_start_ts AS DATE) AS session_start_date
          FROM ((
          WITH _marked AS (
              SELECT
                  *,
                  LAG(event_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_ts,
                  LAG(platform
          ) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_platform
              FROM (SELECT * FROM main.silver_events_parsed WHERE event_date >= '2026-04-07' AND event_date < '2026-04-12') AS source
          ),
          _bounded AS (
              SELECT
                  *,
                  CASE
                      WHEN _prev_ts IS NULL THEN event_ts
                      WHEN epoch_us(event_ts) - epoch_us(_prev_ts) > 30 * 60 * 1000000 THEN event_ts
                      WHEN _prev_platform != platform
           THEN event_ts
                      ELSE NULL
                  END AS _boundary_ts
              FROM _marked
          ),
          _candidate AS (
              SELECT
                  *,
                  MAX(_boundary_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _candidate_root_ts
              FROM _bounded
          ),
          _deadlined AS (
              SELECT
                  *,
                  CASE
                      WHEN _candidate_root_ts IS NULL THEN NULL
                      WHEN CAST(_candidate_root_ts AS TIME) < TIME '00:30:00'
                          THEN CAST(CAST(_candidate_root_ts AS DATE) AS TIMESTAMP) + INTERVAL '1 day'
                      ELSE CAST(CAST(_candidate_root_ts AS DATE) AS TIMESTAMP) + INTERVAL '2 days'
                  END AS _deadline
              FROM _candidate
          )
          SELECT
              *,
              CASE
                  WHEN _candidate_root_ts IS NOT NULL AND event_ts < _deadline THEN _candidate_root_ts
                  ELSE MIN(event_ts) OVER (PARTITION BY device_id, CAST(event_ts AS DATE))
              END AS session_start_ts
          FROM _deadlined
      )) AS __smelt_t2528)
      SELECT
          CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
          device_id,
          session_start_ts,
          session_start_date,
          MIN(event_ts) AS session_start,
          MAX(event_ts) AS session_end,
          COUNT(*) AS event_count,
          ANY_VALUE(platform) AS platform,
          ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
              WHERE utm_campaign IS NOT NULL
                  AND event_ts <= session_start_ts + INTERVAL '5 minutes'
          ) AS utm_campaign
      FROM sessionized
      WHERE event_date
          BETWEEN session_start_date
              AND session_start_date + INTERVAL '1 day'
      GROUP BY device_id, session_start_ts, session_start_date
      HAVING MAX(event_ts) - MIN(event_ts) < INTERVAL '2 days' -- max_session_span: explicit, checkable cap assertion
    
      ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
    COMMIT
    
    -- trigger: NewData { source: "silver.events_parsed" }
    BEGIN
      DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
      INSERT INTO main.silver_sessions SELECT * FROM (
      WITH sessionized AS (
          SELECT
              device_id,
              event_ts,
              event_date,
              platform,
              utm_campaign,
              session_start_ts,
              CAST(session_start_ts AS DATE) AS session_start_date
          FROM ((
          WITH _marked AS (
              SELECT
                  *,
                  LAG(event_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_ts,
                  LAG(platform
          ) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_platform
              FROM (SELECT * FROM main.silver_events_parsed WHERE event_date >= '2026-04-07' AND event_date < '2026-04-12') AS source
          ),
          _bounded AS (
              SELECT
                  *,
                  CASE
                      WHEN _prev_ts IS NULL THEN event_ts
                      WHEN epoch_us(event_ts) - epoch_us(_prev_ts) > 30 * 60 * 1000000 THEN event_ts
                      WHEN _prev_platform != platform
           THEN event_ts
                      ELSE NULL
                  END AS _boundary_ts
              FROM _marked
          ),
          _candidate AS (
              SELECT
                  *,
                  MAX(_boundary_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _candidate_root_ts
              FROM _bounded
          ),
          _deadlined AS (
              SELECT
                  *,
                  CASE
                      WHEN _candidate_root_ts IS NULL THEN NULL
                      WHEN CAST(_candidate_root_ts AS TIME) < TIME '00:30:00'
                          THEN CAST(CAST(_candidate_root_ts AS DATE) AS TIMESTAMP) + INTERVAL '1 day'
                      ELSE CAST(CAST(_candidate_root_ts AS DATE) AS TIMESTAMP) + INTERVAL '2 days'
                  END AS _deadline
              FROM _candidate
          )
          SELECT
              *,
              CASE
                  WHEN _candidate_root_ts IS NOT NULL AND event_ts < _deadline THEN _candidate_root_ts
                  ELSE MIN(event_ts) OVER (PARTITION BY device_id, CAST(event_ts AS DATE))
              END AS session_start_ts
          FROM _deadlined
      )) AS __smelt_t2528)
      SELECT
          CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
          device_id,
          session_start_ts,
          session_start_date,
          MIN(event_ts) AS session_start,
          MAX(event_ts) AS session_end,
          COUNT(*) AS event_count,
          ANY_VALUE(platform) AS platform,
          ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
              WHERE utm_campaign IS NOT NULL
                  AND event_ts <= session_start_ts + INTERVAL '5 minutes'
          ) AS utm_campaign
      FROM sessionized
      WHERE event_date
          BETWEEN session_start_date
              AND session_start_date + INTERVAL '1 day'
      GROUP BY device_id, session_start_ts, session_start_date
      HAVING MAX(event_ts) - MIN(event_ts) < INTERVAL '2 days' -- max_session_span: explicit, checkable cap assertion
    
      ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
    COMMIT
    ```

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
```sql
-- trigger: Backfill
BEGIN
  DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
  INSERT INTO main.silver_sessions SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
COMMIT

-- trigger: NewData { source: "silver.events_parsed" }
BEGIN
  DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
  INSERT INTO main.silver_sessions SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
COMMIT
```

??? example "Full emitted SQL — `smelt explain silver.sessions --show-sql --period 2026-05-04..2026-05-05`"

    <!-- smelt-generate: explain silver.sessions --show-sql --json --period 2026-05-04..2026-05-05 -->
    ```sql
    -- trigger: Backfill
    BEGIN
      DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
      INSERT INTO main.silver_sessions SELECT * FROM (
      WITH sessionized AS (
          SELECT
              device_id,
              event_ts,
              event_date,
              platform,
              utm_campaign,
              session_start_ts,
              CAST(session_start_ts AS DATE) AS session_start_date
          FROM ((
          WITH _marked AS (
              SELECT
                  *,
                  LAG(event_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_ts,
                  LAG(platform
          ) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_platform
              FROM (SELECT * FROM main.silver_events_parsed WHERE event_date >= '2026-05-01' AND event_date < '2026-05-06') AS source
          ),
          _bounded AS (
              SELECT
                  *,
                  CASE
                      WHEN _prev_ts IS NULL THEN event_ts
                      WHEN epoch_us(event_ts) - epoch_us(_prev_ts) > 30 * 60 * 1000000 THEN event_ts
                      WHEN _prev_platform != platform
           THEN event_ts
                      ELSE NULL
                  END AS _boundary_ts
              FROM _marked
          ),
          _candidate AS (
              SELECT
                  *,
                  MAX(_boundary_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _candidate_root_ts
              FROM _bounded
          ),
          _deadlined AS (
              SELECT
                  *,
                  CASE
                      WHEN _candidate_root_ts IS NULL THEN NULL
                      WHEN CAST(_candidate_root_ts AS TIME) < TIME '00:30:00'
                          THEN CAST(CAST(_candidate_root_ts AS DATE) AS TIMESTAMP) + INTERVAL '1 day'
                      ELSE CAST(CAST(_candidate_root_ts AS DATE) AS TIMESTAMP) + INTERVAL '2 days'
                  END AS _deadline
              FROM _candidate
          )
          SELECT
              *,
              CASE
                  WHEN _candidate_root_ts IS NOT NULL AND event_ts < _deadline THEN _candidate_root_ts
                  ELSE MIN(event_ts) OVER (PARTITION BY device_id, CAST(event_ts AS DATE))
              END AS session_start_ts
          FROM _deadlined
      )) AS __smelt_t2528)
      SELECT
          CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
          device_id,
          session_start_ts,
          session_start_date,
          MIN(event_ts) AS session_start,
          MAX(event_ts) AS session_end,
          COUNT(*) AS event_count,
          ANY_VALUE(platform) AS platform,
          ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
              WHERE utm_campaign IS NOT NULL
                  AND event_ts <= session_start_ts + INTERVAL '5 minutes'
          ) AS utm_campaign
      FROM sessionized
      WHERE event_date
          BETWEEN session_start_date
              AND session_start_date + INTERVAL '1 day'
      GROUP BY device_id, session_start_ts, session_start_date
      HAVING MAX(event_ts) - MIN(event_ts) < INTERVAL '2 days' -- max_session_span: explicit, checkable cap assertion
    
      ) AS _smelt_output_clamp WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
    COMMIT
    
    -- trigger: NewData { source: "silver.events_parsed" }
    BEGIN
      DELETE FROM main.silver_sessions WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
      INSERT INTO main.silver_sessions SELECT * FROM (
      WITH sessionized AS (
          SELECT
              device_id,
              event_ts,
              event_date,
              platform,
              utm_campaign,
              session_start_ts,
              CAST(session_start_ts AS DATE) AS session_start_date
          FROM ((
          WITH _marked AS (
              SELECT
                  *,
                  LAG(event_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_ts,
                  LAG(platform
          ) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _prev_platform
              FROM (SELECT * FROM main.silver_events_parsed WHERE event_date >= '2026-05-01' AND event_date < '2026-05-06') AS source
          ),
          _bounded AS (
              SELECT
                  *,
                  CASE
                      WHEN _prev_ts IS NULL THEN event_ts
                      WHEN epoch_us(event_ts) - epoch_us(_prev_ts) > 30 * 60 * 1000000 THEN event_ts
                      WHEN _prev_platform != platform
           THEN event_ts
                      ELSE NULL
                  END AS _boundary_ts
              FROM _marked
          ),
          _candidate AS (
              SELECT
                  *,
                  MAX(_boundary_ts) OVER (
                      PARTITION BY device_id ORDER BY event_ts
                      RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW  -- max_lookback
                  ) AS _candidate_root_ts
              FROM _bounded
          ),
          _deadlined AS (
              SELECT
                  *,
                  CASE
                      WHEN _candidate_root_ts IS NULL THEN NULL
                      WHEN CAST(_candidate_root_ts AS TIME) < TIME '00:30:00'
                          THEN CAST(CAST(_candidate_root_ts AS DATE) AS TIMESTAMP) + INTERVAL '1 day'
                      ELSE CAST(CAST(_candidate_root_ts AS DATE) AS TIMESTAMP) + INTERVAL '2 days'
                  END AS _deadline
              FROM _candidate
          )
          SELECT
              *,
              CASE
                  WHEN _candidate_root_ts IS NOT NULL AND event_ts < _deadline THEN _candidate_root_ts
                  ELSE MIN(event_ts) OVER (PARTITION BY device_id, CAST(event_ts AS DATE))
              END AS session_start_ts
          FROM _deadlined
      )) AS __smelt_t2528)
      SELECT
          CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
          device_id,
          session_start_ts,
          session_start_date,
          MIN(event_ts) AS session_start,
          MAX(event_ts) AS session_end,
          COUNT(*) AS event_count,
          ANY_VALUE(platform) AS platform,
          ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
              WHERE utm_campaign IS NOT NULL
                  AND event_ts <= session_start_ts + INTERVAL '5 minutes'
          ) AS utm_campaign
      FROM sessionized
      WHERE event_date
          BETWEEN session_start_date
              AND session_start_date + INTERVAL '1 day'
      GROUP BY device_id, session_start_ts, session_start_date
      HAVING MAX(event_ts) - MIN(event_ts) < INTERVAL '2 days' -- max_session_span: explicit, checkable cap assertion
    
      ) AS _smelt_output_clamp WHERE session_start_date >= '2026-05-03' AND session_start_date < '2026-05-05'
    COMMIT
    ```

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
```sql
-- trigger: Backfill
BEGIN
  DELETE FROM main.silver_sessions_chained WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
  INSERT INTO main.silver_sessions_chained SELECT * FROM (
  -- … model SELECT body (see the full SQL below) …

  ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
COMMIT

-- trigger: NewData { source: "silver.events_parsed" }
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

??? example "Full emitted SQL — `smelt explain silver.sessions_chained --show-sql --period 2026-04-10..2026-04-11`"

    <!-- smelt-generate: explain silver.sessions_chained --show-sql --json --period 2026-04-10..2026-04-11 -->
    ```sql
    -- trigger: Backfill
    BEGIN
      DELETE FROM main.silver_sessions_chained WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
      INSERT INTO main.silver_sessions_chained SELECT * FROM (
      WITH events AS (
          SELECT device_id, event_ts, event_date, platform, utm_campaign
          FROM main.silver_events_parsed
      ),
      _marked AS (
          SELECT
              *,
              LAG(event_ts) OVER (
                  PARTITION BY device_id ORDER BY event_ts
                  RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW
              ) AS _prev_ts,
              LAG(platform) OVER (
                  PARTITION BY device_id ORDER BY event_ts
                  RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW
              ) AS _prev_platform
          FROM events
      ),
      _bounded AS (
          SELECT
              *,
              CASE
                  WHEN _prev_ts IS NULL THEN event_ts
                  WHEN epoch_us(event_ts) - epoch_us(_prev_ts) > 30 * 60 * 1000000 THEN event_ts
                  WHEN _prev_platform != platform THEN event_ts
                  ELSE NULL
              END AS _boundary_ts
          FROM _marked
      ),
      _candidate AS (
          SELECT
              *,
              MAX(_boundary_ts) OVER (
                  PARTITION BY device_id ORDER BY event_ts
                  ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
              ) AS _local_root_ts
          FROM _bounded
      ),
      _with_self AS (
          SELECT
              _candidate.device_id,
              _candidate.event_ts,
              _candidate.event_date,
              _candidate.platform,
              _candidate.utm_campaign,
              _candidate._local_root_ts,
              (
                  SELECT session_start_ts FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_root_ts,
              (
                  SELECT session_start FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_session_start,
              (
                  SELECT event_count FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_event_count,
              (
                  SELECT utm_campaign FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_utm_campaign,
              (
                  SELECT session_end FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_session_end
          FROM _candidate
      ),
      _anchored AS (
          SELECT
              *,
              CASE
                  WHEN _open_root_ts IS NOT NULL
                       AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
                  THEN _open_root_ts
                  ELSE _local_root_ts
              END AS _anchor_ts,
              CASE
                  WHEN _open_root_ts IS NOT NULL
                       AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
                  THEN _open_session_start
              END AS _anchor_seed_session_start,
              CASE
                  WHEN _open_root_ts IS NOT NULL
                       AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
                  THEN _open_event_count
              END AS _anchor_seed_event_count,
              CASE
                  WHEN _open_root_ts IS NOT NULL
                       AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
                  THEN _open_utm_campaign
              END AS _anchor_seed_utm_campaign
          FROM _with_self
      ),
      _bucketed AS (
          SELECT
              *,
              CAST(FLOOR(DATE_DIFF('day', CAST(_anchor_ts AS DATE), event_date) / 2.0) AS BIGINT) AS _epoch_bucket
          FROM _anchored
      ),
      final AS (
          SELECT
              device_id,
              event_ts,
              event_date,
              platform,
              utm_campaign,
              CASE
                  WHEN _epoch_bucket = 0 THEN _anchor_ts
                  ELSE MIN(event_ts) OVER (PARTITION BY device_id, _local_root_ts, _epoch_bucket)
              END AS session_start_ts,
              CASE WHEN _epoch_bucket = 0 THEN _anchor_seed_session_start END AS _seed_session_start,
              CASE WHEN _epoch_bucket = 0 THEN _anchor_seed_event_count END AS _seed_event_count,
              CASE WHEN _epoch_bucket = 0 THEN _anchor_seed_utm_campaign END AS _seed_utm_campaign
          FROM _bucketed
      ),
      sessionized AS (
          SELECT
              *,
              CAST(session_start_ts AS DATE) AS session_start_date
          FROM final
      ),
      aggregated AS (
          SELECT
              CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
              device_id,
              session_start_ts,
              session_start_date,
              COALESCE(MAX(_seed_session_start), MIN(event_ts)) AS session_start,
              MAX(event_ts) AS session_end,
              COUNT(*) + COALESCE(MAX(_seed_event_count), 0) AS event_count,
              ANY_VALUE(platform) AS platform,
              COALESCE(
                  MAX(_seed_utm_campaign),
                  ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
                      WHERE utm_campaign IS NOT NULL
                          AND event_ts <= session_start_ts + INTERVAL '5 minutes'
                  )
              ) AS utm_campaign
          FROM sessionized
          WHERE event_date
              BETWEEN session_start_date
                  AND session_start_date + INTERVAL '1 day'
          GROUP BY device_id, session_start_ts, session_start_date
          HAVING MAX(event_ts) - COALESCE(MAX(_seed_session_start), MIN(event_ts)) < INTERVAL '2 days' -- root-anchored cap: explicit, checkable assertion
      )
      SELECT
          session_id,
          device_id,
          session_start_ts,
          session_start_date,
          session_start,
          session_end,
          event_count,
          platform,
          utm_campaign
      FROM aggregated
    
      ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
    COMMIT
    
    -- trigger: NewData { source: "silver.events_parsed" }
    BEGIN
      DELETE FROM main.silver_sessions_chained WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
      INSERT INTO main.silver_sessions_chained SELECT * FROM (
      WITH events AS (
          SELECT device_id, event_ts, event_date, platform, utm_campaign
          FROM main.silver_events_parsed
      ),
      _marked AS (
          SELECT
              *,
              LAG(event_ts) OVER (
                  PARTITION BY device_id ORDER BY event_ts
                  RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW
              ) AS _prev_ts,
              LAG(platform) OVER (
                  PARTITION BY device_id ORDER BY event_ts
                  RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW
              ) AS _prev_platform
          FROM events
      ),
      _bounded AS (
          SELECT
              *,
              CASE
                  WHEN _prev_ts IS NULL THEN event_ts
                  WHEN epoch_us(event_ts) - epoch_us(_prev_ts) > 30 * 60 * 1000000 THEN event_ts
                  WHEN _prev_platform != platform THEN event_ts
                  ELSE NULL
              END AS _boundary_ts
          FROM _marked
      ),
      _candidate AS (
          SELECT
              *,
              MAX(_boundary_ts) OVER (
                  PARTITION BY device_id ORDER BY event_ts
                  ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
              ) AS _local_root_ts
          FROM _bounded
      ),
      _with_self AS (
          SELECT
              _candidate.device_id,
              _candidate.event_ts,
              _candidate.event_date,
              _candidate.platform,
              _candidate.utm_campaign,
              _candidate._local_root_ts,
              (
                  SELECT session_start_ts FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_root_ts,
              (
                  SELECT session_start FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_session_start,
              (
                  SELECT event_count FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_event_count,
              (
                  SELECT utm_campaign FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_utm_campaign,
              (
                  SELECT session_end FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_session_end
          FROM _candidate
      ),
      _anchored AS (
          SELECT
              *,
              CASE
                  WHEN _open_root_ts IS NOT NULL
                       AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
                  THEN _open_root_ts
                  ELSE _local_root_ts
              END AS _anchor_ts,
              CASE
                  WHEN _open_root_ts IS NOT NULL
                       AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
                  THEN _open_session_start
              END AS _anchor_seed_session_start,
              CASE
                  WHEN _open_root_ts IS NOT NULL
                       AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
                  THEN _open_event_count
              END AS _anchor_seed_event_count,
              CASE
                  WHEN _open_root_ts IS NOT NULL
                       AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
                  THEN _open_utm_campaign
              END AS _anchor_seed_utm_campaign
          FROM _with_self
      ),
      _bucketed AS (
          SELECT
              *,
              CAST(FLOOR(DATE_DIFF('day', CAST(_anchor_ts AS DATE), event_date) / 2.0) AS BIGINT) AS _epoch_bucket
          FROM _anchored
      ),
      final AS (
          SELECT
              device_id,
              event_ts,
              event_date,
              platform,
              utm_campaign,
              CASE
                  WHEN _epoch_bucket = 0 THEN _anchor_ts
                  ELSE MIN(event_ts) OVER (PARTITION BY device_id, _local_root_ts, _epoch_bucket)
              END AS session_start_ts,
              CASE WHEN _epoch_bucket = 0 THEN _anchor_seed_session_start END AS _seed_session_start,
              CASE WHEN _epoch_bucket = 0 THEN _anchor_seed_event_count END AS _seed_event_count,
              CASE WHEN _epoch_bucket = 0 THEN _anchor_seed_utm_campaign END AS _seed_utm_campaign
          FROM _bucketed
      ),
      sessionized AS (
          SELECT
              *,
              CAST(session_start_ts AS DATE) AS session_start_date
          FROM final
      ),
      aggregated AS (
          SELECT
              CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
              device_id,
              session_start_ts,
              session_start_date,
              COALESCE(MAX(_seed_session_start), MIN(event_ts)) AS session_start,
              MAX(event_ts) AS session_end,
              COUNT(*) + COALESCE(MAX(_seed_event_count), 0) AS event_count,
              ANY_VALUE(platform) AS platform,
              COALESCE(
                  MAX(_seed_utm_campaign),
                  ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
                      WHERE utm_campaign IS NOT NULL
                          AND event_ts <= session_start_ts + INTERVAL '5 minutes'
                  )
              ) AS utm_campaign
          FROM sessionized
          WHERE event_date
              BETWEEN session_start_date
                  AND session_start_date + INTERVAL '1 day'
          GROUP BY device_id, session_start_ts, session_start_date
          HAVING MAX(event_ts) - COALESCE(MAX(_seed_session_start), MIN(event_ts)) < INTERVAL '2 days' -- root-anchored cap: explicit, checkable assertion
      )
      SELECT
          session_id,
          device_id,
          session_start_ts,
          session_start_date,
          session_start,
          session_end,
          event_count,
          platform,
          utm_campaign
      FROM aggregated
    
      ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
    COMMIT
    
    -- trigger: NewData { source: "silver.sessions_chained" }
    BEGIN
      DELETE FROM main.silver_sessions_chained WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
      INSERT INTO main.silver_sessions_chained SELECT * FROM (
      WITH events AS (
          SELECT device_id, event_ts, event_date, platform, utm_campaign
          FROM main.silver_events_parsed
      ),
      _marked AS (
          SELECT
              *,
              LAG(event_ts) OVER (
                  PARTITION BY device_id ORDER BY event_ts
                  RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW
              ) AS _prev_ts,
              LAG(platform) OVER (
                  PARTITION BY device_id ORDER BY event_ts
                  RANGE BETWEEN INTERVAL '2 days' PRECEDING AND CURRENT ROW
              ) AS _prev_platform
          FROM events
      ),
      _bounded AS (
          SELECT
              *,
              CASE
                  WHEN _prev_ts IS NULL THEN event_ts
                  WHEN epoch_us(event_ts) - epoch_us(_prev_ts) > 30 * 60 * 1000000 THEN event_ts
                  WHEN _prev_platform != platform THEN event_ts
                  ELSE NULL
              END AS _boundary_ts
          FROM _marked
      ),
      _candidate AS (
          SELECT
              *,
              MAX(_boundary_ts) OVER (
                  PARTITION BY device_id ORDER BY event_ts
                  ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
              ) AS _local_root_ts
          FROM _bounded
      ),
      _with_self AS (
          SELECT
              _candidate.device_id,
              _candidate.event_ts,
              _candidate.event_date,
              _candidate.platform,
              _candidate.utm_campaign,
              _candidate._local_root_ts,
              (
                  SELECT session_start_ts FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_root_ts,
              (
                  SELECT session_start FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_session_start,
              (
                  SELECT event_count FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_event_count,
              (
                  SELECT utm_campaign FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_utm_campaign,
              (
                  SELECT session_end FROM main.silver_sessions_chained
                  WHERE device_id = _candidate.device_id
                      AND platform = _candidate.platform
                      AND session_start_date >= CAST(_candidate._local_root_ts AS DATE) - INTERVAL '2 days'
                      AND session_start_date < CAST(_candidate._local_root_ts AS DATE)
                  ORDER BY session_end DESC LIMIT 1
              ) AS _open_session_end
          FROM _candidate
      ),
      _anchored AS (
          SELECT
              *,
              CASE
                  WHEN _open_root_ts IS NOT NULL
                       AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
                  THEN _open_root_ts
                  ELSE _local_root_ts
              END AS _anchor_ts,
              CASE
                  WHEN _open_root_ts IS NOT NULL
                       AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
                  THEN _open_session_start
              END AS _anchor_seed_session_start,
              CASE
                  WHEN _open_root_ts IS NOT NULL
                       AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
                  THEN _open_event_count
              END AS _anchor_seed_event_count,
              CASE
                  WHEN _open_root_ts IS NOT NULL
                       AND epoch_us(_local_root_ts) - epoch_us(_open_session_end) <= 30 * 60 * 1000000
                  THEN _open_utm_campaign
              END AS _anchor_seed_utm_campaign
          FROM _with_self
      ),
      _bucketed AS (
          SELECT
              *,
              CAST(FLOOR(DATE_DIFF('day', CAST(_anchor_ts AS DATE), event_date) / 2.0) AS BIGINT) AS _epoch_bucket
          FROM _anchored
      ),
      final AS (
          SELECT
              device_id,
              event_ts,
              event_date,
              platform,
              utm_campaign,
              CASE
                  WHEN _epoch_bucket = 0 THEN _anchor_ts
                  ELSE MIN(event_ts) OVER (PARTITION BY device_id, _local_root_ts, _epoch_bucket)
              END AS session_start_ts,
              CASE WHEN _epoch_bucket = 0 THEN _anchor_seed_session_start END AS _seed_session_start,
              CASE WHEN _epoch_bucket = 0 THEN _anchor_seed_event_count END AS _seed_event_count,
              CASE WHEN _epoch_bucket = 0 THEN _anchor_seed_utm_campaign END AS _seed_utm_campaign
          FROM _bucketed
      ),
      sessionized AS (
          SELECT
              *,
              CAST(session_start_ts AS DATE) AS session_start_date
          FROM final
      ),
      aggregated AS (
          SELECT
              CONCAT(CAST(device_id AS VARCHAR), '-', CAST(session_start_ts AS VARCHAR)) AS session_id,
              device_id,
              session_start_ts,
              session_start_date,
              COALESCE(MAX(_seed_session_start), MIN(event_ts)) AS session_start,
              MAX(event_ts) AS session_end,
              COUNT(*) + COALESCE(MAX(_seed_event_count), 0) AS event_count,
              ANY_VALUE(platform) AS platform,
              COALESCE(
                  MAX(_seed_utm_campaign),
                  ARG_MAX(utm_campaign, -epoch_us(event_ts)) FILTER (
                      WHERE utm_campaign IS NOT NULL
                          AND event_ts <= session_start_ts + INTERVAL '5 minutes'
                  )
              ) AS utm_campaign
          FROM sessionized
          WHERE event_date
              BETWEEN session_start_date
                  AND session_start_date + INTERVAL '1 day'
          GROUP BY device_id, session_start_ts, session_start_date
          HAVING MAX(event_ts) - COALESCE(MAX(_seed_session_start), MIN(event_ts)) < INTERVAL '2 days' -- root-anchored cap: explicit, checkable assertion
      )
      SELECT
          session_id,
          device_id,
          session_start_ts,
          session_start_date,
          session_start,
          session_end,
          event_count,
          platform,
          utm_campaign
      FROM aggregated
    
      ) AS _smelt_output_clamp WHERE session_start_date >= '2026-04-09' AND session_start_date < '2026-04-11'
    COMMIT
    ```

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
