---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    country,
    region,
    category,
    is_verified
FROM smelt.ref('sql_l3_246')
WHERE score >= 50
