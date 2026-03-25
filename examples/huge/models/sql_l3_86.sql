---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    session_id,
    country,
    updated_at
FROM smelt.ref('sql_l2_117')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_58') WHERE event_type = 'purchase'
)
