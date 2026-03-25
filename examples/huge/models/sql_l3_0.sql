---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    country,
    price,
    is_active
FROM smelt.ref('sql_l2_65')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_330') WHERE platform = 'web'
)
