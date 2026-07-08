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
    SELECT quantity, ip_address, rating
    FROM smelt.events
    WHERE is_active = true
)
SELECT
    b.quantity,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.events j ON b.user_id = j.user_id
GROUP BY b.quantity
