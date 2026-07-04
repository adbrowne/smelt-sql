---
materialization: table
timeseries:
  event_time_column: event_ts
  partition_column: event_date
  granularity: day
refresh: batched
horizon_ceiling: '30 days'
---
-- A 2-hour `RANGE BETWEEN INTERVAL` lookback derives a horizon far inside
-- the declared 30-day `horizon_ceiling` — smelt emits no warning here. The
-- clamp always uses the derived (2-hour) reach, regardless of the ceiling.
SELECT
    date_trunc('day', e.event_ts)::DATE AS event_date,
    e.user_id,
    e.amount,
    SUM(e.amount) OVER (
        PARTITION BY e.user_id
        ORDER BY e.event_ts
        RANGE BETWEEN INTERVAL '2 hours' PRECEDING AND CURRENT ROW
    ) AS rolling_amount
FROM smelt.events e
