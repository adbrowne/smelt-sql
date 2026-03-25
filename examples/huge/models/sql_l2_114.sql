---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    ip_address,
    category,
    quantity,
    segment
FROM smelt.ref('sql_l1_1')
WHERE category IS NOT NULL
