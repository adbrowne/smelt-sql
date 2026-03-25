---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    transaction_id,
    SUM(quantity) AS val_1,
    SUM(amount) AS val_2
FROM smelt.ref('sql_l2_173')
GROUP BY transaction_id
HAVING COUNT(*) > 10
