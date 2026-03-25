---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    category,
    rating,
    event_type
FROM smelt.ref('subscriptions')
WHERE category IS NOT NULL
