---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.tier,
    b.profit,
    c.status,
    c.page_path
FROM smelt.ref('sql_l2_239') a
INNER JOIN smelt.ref('py_l2_281') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l2_239') c ON a.user_id = c.user_id
