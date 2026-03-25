---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.region,
    a.platform,
    b.segment
FROM smelt.ref('py_l2_457') a
INNER JOIN smelt.ref('sql_l2_191') b ON a.user_id = b.user_id
