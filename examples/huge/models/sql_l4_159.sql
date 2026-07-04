---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.email_domain,
    a.session_id,
    b.is_verified
FROM smelt.sql_l3_202 a
INNER JOIN smelt.sql_l3_202 b ON a.user_id = b.user_id
