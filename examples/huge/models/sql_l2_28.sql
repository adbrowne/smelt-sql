---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    rating,
    created_at,
    cost,
    event_date
FROM smelt.ref('sql_l1_115')
WHERE score >= 50
