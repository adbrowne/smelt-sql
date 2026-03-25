---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    referrer,
    device_type,
    category
FROM smelt.ref('sql_l3_227')
WHERE is_active = true
