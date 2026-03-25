---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    os_name,
    transaction_id,
    email_domain
FROM smelt.ref('py_l2_294')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_385') WHERE created_at >= '2024-01-01'
)
