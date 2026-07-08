---
materialization: table
timeseries:
  event_time_column: event_ts
  partition_column: event_date
  granularity: day
refresh: incremental
grain: partition
horizon_ceiling: '1 hour'
---
-- A 2-hour `RANGE BETWEEN INTERVAL` lookback derives a horizon that exceeds
-- the declared 1-hour `horizon_ceiling` — smelt warns at compile time, but
-- the clamp still uses the derived (2-hour) reach; the ceiling never
-- narrows it.
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
