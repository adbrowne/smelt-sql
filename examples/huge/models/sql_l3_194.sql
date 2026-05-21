---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.created_at,
    b.category,
    c.platform,
    c.cohort_date
FROM smelt.sql_l2_227 a
INNER JOIN smelt.sql_l2_168 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_227 c ON a.user_id = c.user_id

