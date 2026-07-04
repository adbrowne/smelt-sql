---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    plan_type,
    platform,
    score
FROM smelt.shipments
WHERE user_id IN (
    SELECT user_id FROM smelt.shipments WHERE status = 'active'
)
