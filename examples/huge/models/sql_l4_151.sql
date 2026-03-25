---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    category,
    is_active,
    channel,
    created_at
FROM smelt.ref('sql_l3_221')
WHERE is_active = true
