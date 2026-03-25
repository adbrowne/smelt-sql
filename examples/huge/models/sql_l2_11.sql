---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.device_type,
    b.tier,
    c.browser,
    c.campaign_id
FROM smelt.ref('py_l1_335') a
INNER JOIN smelt.ref('sql_l1_30') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l1_335') c ON a.user_id = c.user_id
