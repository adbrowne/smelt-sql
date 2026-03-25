---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    ip_address,
    channel,
    transaction_id
FROM smelt.ref('py_l1_437')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l1_437') WHERE is_active = true
)
