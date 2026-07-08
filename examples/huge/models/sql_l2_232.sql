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
    a.order_id,
    a.page_path,
    b.campaign_id
FROM smelt.sql_l1_145 a
INNER JOIN smelt.sql_l1_145 b ON a.user_id = b.user_id
