---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    platform,
    is_active,
    page_path,
    region
FROM smelt.ref('sql_l3_165')
WHERE quantity > 0
