---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    browser,
    product_id,
    RANK() OVER (PARTITION BY browser ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l1_204')
