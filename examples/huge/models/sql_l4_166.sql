---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.page_path,
    a.user_id,
    b.session_id
FROM smelt.models.sql_l3_3 a
LEFT JOIN smelt.models.sql_l3_3 b ON a.user_id = b.user_id

