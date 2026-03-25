---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.campaign_id,
    b.browser,
    c.cost,
    c.price
FROM smelt.ref('py_l2_313') a
INNER JOIN smelt.ref('sql_l2_71') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l2_267') c ON a.user_id = c.user_id
