---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cost,
    email_domain,
    amount
FROM smelt.ref('py_l2_473')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_363') WHERE event_type = 'purchase'
)
