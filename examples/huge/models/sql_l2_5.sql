---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.segment,
    a.cohort_date,
    b.category
FROM smelt.models.sql_l1_182 a
INNER JOIN smelt.models.sql_l1_182 b ON a.user_id = b.user_id

