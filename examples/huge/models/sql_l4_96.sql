---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    segment,
    is_verified,
    referrer
FROM smelt.ref('sql_l3_105')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_105') WHERE score >= 50
)
