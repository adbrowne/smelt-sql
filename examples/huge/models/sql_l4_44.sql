---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.amount,
    a.discount,
    b.status
FROM smelt.ref('sql_l3_90') a
LEFT JOIN smelt.ref('sql_l3_90') b ON a.user_id = b.user_id
