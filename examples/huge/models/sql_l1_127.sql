---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    amount,
    category,
    event_time,
    profit
FROM smelt.ref('categories')
WHERE amount > 0
