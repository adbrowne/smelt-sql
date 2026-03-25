---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    region,
    product_id,
    RANK() OVER (PARTITION BY region ORDER BY created_at) AS win_val
FROM smelt.ref('users')
