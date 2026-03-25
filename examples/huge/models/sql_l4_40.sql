---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    revenue,
    status,
    rating,
    tier
FROM smelt.ref('py_l3_352')
WHERE event_type = 'purchase'
