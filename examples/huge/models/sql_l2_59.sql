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
    a.page_path,
    b.updated_at
FROM smelt.models.sql_l1_158 a
INNER JOIN smelt.models.sql_l1_158 b ON a.user_id = b.user_id

