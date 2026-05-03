---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.cohort_date,
    b.channel,
    c.segment,
    c.region
FROM smelt.sql_l1_247 a
INNER JOIN smelt.sql_l1_186 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l1_33 c ON a.user_id = c.user_id

