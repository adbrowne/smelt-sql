---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    ip_address,
    event_date,
    region
FROM smelt.ref('transactions')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('transactions') WHERE status = 'active'
)
