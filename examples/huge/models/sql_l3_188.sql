---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.session_id,
    a.duration_seconds,
    b.event_time
FROM smelt.ref('py_l2_358') a
LEFT JOIN smelt.ref('sql_l2_139') b ON a.user_id = b.user_id
