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
    AVG(price) AS val_1,
    SUM(amount) AS val_2
FROM smelt.models.sql_l3_130
GROUP BY cohort_date
HAVING COUNT(*) > 10

