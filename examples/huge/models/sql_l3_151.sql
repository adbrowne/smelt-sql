---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    is_verified,
    CASE
        WHEN amount > 1000 THEN 'high'
        WHEN amount > 100 THEN 'medium'
        ELSE 'low'
    END AS value_tier
FROM smelt.ref('py_l2_277')
