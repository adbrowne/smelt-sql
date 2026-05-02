---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.channel,
    a.is_verified,
    b.cohort_date
FROM smelt.models.sql_l1_104 a
LEFT JOIN smelt.models.sql_l1_154 b ON a.user_id = b.user_id

