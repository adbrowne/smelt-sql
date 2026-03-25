---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.profit,
    a.transaction_id,
    b.amount
FROM smelt.ref('sql_l3_169') a
INNER JOIN smelt.ref('sql_l3_122') b ON a.user_id = b.user_id
