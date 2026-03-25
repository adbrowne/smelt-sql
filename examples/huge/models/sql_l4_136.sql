---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    os_name,
    ip_address,
    is_active
FROM smelt.ref('sql_l3_119')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_119') WHERE quantity > 0
)
