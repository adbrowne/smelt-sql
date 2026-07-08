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
    SELECT tier, revenue, event_date
    FROM smelt.sql_l3_189
    WHERE platform = 'web'
)
SELECT
    b.tier,
    AVG(duration_seconds) AS agg_val
FROM base b
INNER JOIN smelt.sql_l3_50 j ON b.user_id = j.user_id
GROUP BY b.tier
