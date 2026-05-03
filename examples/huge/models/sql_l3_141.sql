---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.cohort_date,
    b.session_id,
    c.category,
    c.region
FROM smelt.sql_l2_175 a
INNER JOIN smelt.sql_l2_203 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_198 c ON a.user_id = c.user_id

