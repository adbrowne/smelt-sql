---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    rating,
    channel,
    plan_type,
    user_id
FROM smelt.ref('sql_l3_145')
WHERE score >= 50
