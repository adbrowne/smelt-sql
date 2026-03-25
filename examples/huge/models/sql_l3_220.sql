---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.duration_seconds,
    a.product_id,
    b.status
FROM smelt.ref('py_l2_314') a
INNER JOIN smelt.ref('py_l2_393') b ON a.user_id = b.user_id
