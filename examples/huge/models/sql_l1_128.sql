---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_date,
    product_id,
    referrer
FROM smelt.clicks
WHERE user_id IN (
    SELECT user_id FROM smelt.clicks WHERE status = 'active'
)
