---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.product_id,
    b.platform,
    c.plan_type,
    c.cohort_date
FROM smelt.models.sql_l1_194 a
INNER JOIN smelt.models.sql_l1_171 b ON a.user_id = b.user_id
LEFT JOIN smelt.models.sql_l1_194 c ON a.user_id = c.user_id

