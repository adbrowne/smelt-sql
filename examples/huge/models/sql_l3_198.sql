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
    SUM(quantity) AS val_1,
    SUM(amount) AS val_2
FROM smelt.models.sql_l2_223
GROUP BY cohort_date
HAVING COUNT(*) > 10

