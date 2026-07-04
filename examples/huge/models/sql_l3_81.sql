---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.is_active,
    b.campaign_id,
    c.email_domain,
    c.price
FROM smelt.sql_l2_52 a
INNER JOIN smelt.sql_l2_9 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_87 c ON a.user_id = c.user_id
