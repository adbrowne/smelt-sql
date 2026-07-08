---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.page_path,
    b.email_domain,
    c.device_type,
    c.category
FROM smelt.sql_l2_218 a
INNER JOIN smelt.sql_l2_138 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_218 c ON a.user_id = c.user_id
