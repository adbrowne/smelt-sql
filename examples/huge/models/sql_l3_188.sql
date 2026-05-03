---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.session_id,
    a.duration_seconds,
    b.event_time
FROM smelt.sql_l2_58 a
LEFT JOIN smelt.sql_l2_122 b ON a.user_id = b.user_id

