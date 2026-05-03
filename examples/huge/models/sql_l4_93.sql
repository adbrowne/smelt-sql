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
    a.is_verified,
    b.plan_type
FROM smelt.sql_l3_52 a
LEFT JOIN smelt.sql_l3_66 b ON a.user_id = b.user_id

