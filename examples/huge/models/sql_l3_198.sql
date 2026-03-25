---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cohort_date,
    SUM(quantity) AS val_1,
    SUM(amount) AS val_2
FROM smelt.ref('sql_l2_153')
GROUP BY cohort_date
HAVING COUNT(*) > 10
