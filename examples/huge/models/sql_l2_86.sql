---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    channel,
    amount,
    status,
    discount
FROM smelt.ref('sql_l1_154')
WHERE amount > 0
