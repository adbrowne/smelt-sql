---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cohort_date,
    quantity,
    LAG(amount, 1) OVER (PARTITION BY cohort_date ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_158')
