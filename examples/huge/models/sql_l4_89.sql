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
    a.created_at,
    b.cohort_date
FROM smelt.sql_l3_78 a
INNER JOIN smelt.sql_l3_136 b ON a.user_id = b.user_id

