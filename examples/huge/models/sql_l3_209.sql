---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    ip_address,
    event_date,
    LAG(amount, 1) OVER (PARTITION BY ip_address ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l2_234')
