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
    a.discount,
    a.email_domain,
    b.browser
FROM smelt.sql_l3_171 a
LEFT JOIN smelt.sql_l3_186 b ON a.user_id = b.user_id
