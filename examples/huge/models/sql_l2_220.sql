---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    score,
    email_domain,
    discount
FROM smelt.ref('py_l1_407')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l1_407') WHERE status = 'active'
)
