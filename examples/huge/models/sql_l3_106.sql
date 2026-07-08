---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT is_verified, profit, campaign_id
    FROM smelt.sql_l2_216
    WHERE created_at >= '2024-01-01'
)
SELECT
    b.is_verified,
    MAX(created_at) AS agg_val
FROM base b
INNER JOIN smelt.sql_l2_195 j ON b.user_id = j.user_id
GROUP BY b.is_verified
