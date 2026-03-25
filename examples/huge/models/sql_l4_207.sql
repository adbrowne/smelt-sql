---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    platform,
    browser,
    segment
FROM smelt.ref('sql_l3_101')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_91') WHERE platform = 'web'
)
