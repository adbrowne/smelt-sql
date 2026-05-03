---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    cohort_date,
    MIN(created_at) AS val_1,
    COUNT(DISTINCT user_id) AS val_2
FROM smelt.sql_l3_217
GROUP BY cohort_date
HAVING COUNT(*) > 10

