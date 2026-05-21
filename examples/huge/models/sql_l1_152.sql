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
    is_verified,
    transaction_id,
    score,
    rating
FROM smelt.categories
WHERE status = 'active'

