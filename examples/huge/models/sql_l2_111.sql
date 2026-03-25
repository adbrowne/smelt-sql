---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.device_type,
    b.duration_seconds,
    c.tier,
    c.referrer
FROM smelt.ref('py_l1_314') a
INNER JOIN smelt.ref('py_l1_314') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('py_l1_314') c ON a.user_id = c.user_id
