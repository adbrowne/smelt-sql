---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    amount,
    category,
    event_time,
    profit
FROM smelt.ref('categories')
WHERE amount > 0
