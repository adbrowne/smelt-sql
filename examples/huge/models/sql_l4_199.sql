---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cohort_date,
    AVG(price) AS val_1,
    SUM(amount) AS val_2
FROM smelt.ref('sql_l3_137')
GROUP BY cohort_date
HAVING COUNT(*) > 10
