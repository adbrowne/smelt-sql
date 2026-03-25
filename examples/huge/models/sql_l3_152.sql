---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.channel,
    a.amount,
    b.browser
FROM smelt.ref('py_l2_331') a
INNER JOIN smelt.ref('sql_l2_183') b ON a.user_id = b.user_id
