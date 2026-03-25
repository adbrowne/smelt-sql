---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    price,
    region,
    rating,
    event_time
FROM smelt.ref('transactions')
WHERE amount > 0
