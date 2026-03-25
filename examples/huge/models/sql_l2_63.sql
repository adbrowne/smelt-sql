---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    ip_address,
    channel,
    transaction_id
FROM smelt.ref('sql_l1_67')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_67') WHERE is_active = true
)
