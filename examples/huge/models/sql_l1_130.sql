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
    transaction_id,
    score,
    tier,
    quantity
FROM smelt.clicks
WHERE platform = 'web'

