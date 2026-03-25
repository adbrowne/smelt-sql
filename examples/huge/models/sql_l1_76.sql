---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    score,
    revenue,
    LAG(amount, 1) OVER (PARTITION BY score ORDER BY created_at) AS win_val
FROM smelt.ref('page_views')
