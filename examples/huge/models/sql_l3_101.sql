---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.duration_seconds,
    a.device_type,
    b.updated_at
FROM smelt.ref('sql_l2_198') a
LEFT JOIN smelt.ref('sql_l2_126') b ON a.user_id = b.user_id
