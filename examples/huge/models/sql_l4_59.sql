---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.tier,
    a.ip_address,
    b.amount
FROM smelt.ref('sql_l3_214') a
LEFT JOIN smelt.ref('sql_l3_222') b ON a.user_id = b.user_id
