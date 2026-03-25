---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    discount,
    transaction_id,
    tier,
    is_verified
FROM smelt.ref('sql_l2_240')
WHERE event_type = 'purchase'
