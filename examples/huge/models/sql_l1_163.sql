---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    plan_type,
    cohort_date,
    LAG(amount, 1) OVER (PARTITION BY plan_type ORDER BY created_at) AS win_val
FROM smelt.ref('page_views')
