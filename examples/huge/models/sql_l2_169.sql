---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    updated_at,
    created_at
FROM smelt.ref('py_l1_285')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_67') WHERE country = 'US'
)
