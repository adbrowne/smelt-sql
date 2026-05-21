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
    a.updated_at,
    a.status,
    b.region
FROM smelt.sql_l3_77 a
INNER JOIN smelt.sql_l3_208 b ON a.user_id = b.user_id

