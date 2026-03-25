---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.country,
    a.transaction_id,
    b.status
FROM smelt.ref('py_l1_374') a
LEFT JOIN smelt.ref('sql_l1_188') b ON a.user_id = b.user_id
