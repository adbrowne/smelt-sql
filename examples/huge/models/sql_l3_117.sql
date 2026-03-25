---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    score,
    LAG(amount, 1) OVER (PARTITION BY is_verified ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l2_216')
