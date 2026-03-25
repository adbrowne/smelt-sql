---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    price,
    region,
    rating,
    event_time
FROM smelt.ref('transactions')
WHERE amount > 0
