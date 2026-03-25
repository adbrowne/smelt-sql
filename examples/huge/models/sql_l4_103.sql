---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.plan_type,
    a.event_type,
    b.region
FROM smelt.ref('sql_l3_236') a
LEFT JOIN smelt.ref('sql_l3_236') b ON a.user_id = b.user_id
