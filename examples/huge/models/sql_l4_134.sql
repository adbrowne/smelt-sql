---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cohort_date,
    MIN(created_at) AS val_1,
    COUNT(DISTINCT user_id) AS val_2
FROM smelt.ref('sql_l3_16')
GROUP BY cohort_date
HAVING COUNT(*) > 10
