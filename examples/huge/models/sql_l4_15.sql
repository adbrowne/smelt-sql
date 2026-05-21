---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.category,
    a.is_active,
    b.score
FROM smelt.sql_l3_83 a
INNER JOIN smelt.sql_l3_132 b ON a.user_id = b.user_id

