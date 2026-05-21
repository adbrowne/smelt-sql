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
    a.updated_at,
    b.transaction_id,
    c.cohort_date,
    c.cost
FROM smelt.sql_l3_220 a
INNER JOIN smelt.sql_l3_220 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l3_220 c ON a.user_id = c.user_id

