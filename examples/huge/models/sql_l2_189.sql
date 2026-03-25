---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    referrer,
    device_type,
    score
FROM smelt.ref('sql_l1_46')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_206') WHERE amount > 0
)
