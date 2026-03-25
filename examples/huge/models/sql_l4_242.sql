---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cohort_date,
    MIN(created_at) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.ref('sql_l3_166')
GROUP BY cohort_date
HAVING COUNT(*) > 10
