---
materialization: table
timeseries:
  event_time_column: event_ts
  partition_column: event_date
  granularity: day
incremental:
  enabled: true
---
-- This model uses LAG without a RANGE BETWEEN INTERVAL clause.
-- The planner cannot derive a temporal bound and must refuse it.
SELECT
    date_trunc('day', e.event_ts)::DATE AS event_date,
    e.user_id,
    e.amount,
    LAG(e.amount) OVER (PARTITION BY e.user_id ORDER BY e.event_ts) AS prev_amount
FROM smelt.models.events e
