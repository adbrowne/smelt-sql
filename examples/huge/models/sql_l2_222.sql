---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    device_type,
    session_id
FROM smelt.ref('py_l1_477')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_204') WHERE category IS NOT NULL
)
