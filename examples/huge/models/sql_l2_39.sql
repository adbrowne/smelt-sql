---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    revenue,
    rating,
    cohort_date,
    profit
FROM smelt.ref('sql_l1_32')
WHERE created_at >= '2024-01-01'
