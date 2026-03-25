---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    ip_address,
    created_at,
    LAG(amount, 1) OVER (PARTITION BY ip_address ORDER BY created_at) AS win_val
FROM smelt.ref('page_views')
