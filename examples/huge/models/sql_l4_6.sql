---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.profit,
    a.transaction_id,
    b.amount
FROM smelt.ref('sql_l3_157') a
INNER JOIN smelt.ref('sql_l3_37') b ON a.user_id = b.user_id
