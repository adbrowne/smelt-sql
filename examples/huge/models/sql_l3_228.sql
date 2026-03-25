---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.channel,
    b.referrer,
    c.browser,
    c.product_id
FROM smelt.ref('sql_l2_52') a
INNER JOIN smelt.ref('py_l2_413') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l2_52') c ON a.user_id = c.user_id
