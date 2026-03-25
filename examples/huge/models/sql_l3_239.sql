---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    discount,
    transaction_id,
    tier,
    is_verified
FROM smelt.ref('sql_l2_81')
WHERE event_type = 'purchase'
