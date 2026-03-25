---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.event_date,
    a.region,
    b.browser
FROM smelt.ref('sql_l1_240') a
INNER JOIN smelt.ref('sql_l1_139') b ON a.user_id = b.user_id
