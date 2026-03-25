---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_verified,
    event_type,
    category
FROM smelt.ref('sql_l3_95')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_17') WHERE is_active = true
)
