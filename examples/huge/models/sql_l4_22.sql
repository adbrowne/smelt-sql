---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    product_id,
    country,
    RANK() OVER (PARTITION BY product_id ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_76')
