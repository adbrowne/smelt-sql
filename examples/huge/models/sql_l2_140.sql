---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.event_time,
    a.plan_type,
    b.browser
FROM smelt.sql_l1_135 a
INNER JOIN smelt.sql_l1_177 b ON a.user_id = b.user_id

