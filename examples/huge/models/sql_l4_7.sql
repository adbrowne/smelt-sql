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
    a.os_name,
    a.discount,
    b.campaign_id
FROM smelt.sql_l3_58 a
INNER JOIN smelt.sql_l3_188 b ON a.user_id = b.user_id
