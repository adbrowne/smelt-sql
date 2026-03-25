---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    created_at,
    tier,
    is_verified
FROM smelt.ref('sql_l2_144')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_263') WHERE category IS NOT NULL
)
