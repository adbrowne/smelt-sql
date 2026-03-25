---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cohort_date,
    category,
    os_name,
    product_id
FROM smelt.ref('py_l1_274')
WHERE created_at >= '2024-01-01'
