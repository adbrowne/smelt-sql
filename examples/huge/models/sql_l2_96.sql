---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT platform, ip_address, category
    FROM smelt.sql_l1_76
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.platform,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.sql_l1_128 j ON b.user_id = j.user_id
GROUP BY b.platform
