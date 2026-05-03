---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    revenue,
    cohort_date,
    event_date
FROM smelt.sql_l2_33
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_185 WHERE status = 'active'
)

