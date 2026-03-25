---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.updated_at,
    a.status,
    b.region
FROM smelt.ref('py_l3_481') a
INNER JOIN smelt.ref('py_l3_481') b ON a.user_id = b.user_id
