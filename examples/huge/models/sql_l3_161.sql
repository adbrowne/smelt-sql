---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    platform,
    product_id,
    RANK() OVER (PARTITION BY platform ORDER BY created_at) AS win_val
FROM smelt.ref('py_l2_418')
