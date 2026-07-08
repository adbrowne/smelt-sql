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
    SELECT segment, cost, country
    FROM smelt.transactions
    WHERE platform = 'web'
)
SELECT
    b.segment,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.transactions j ON b.user_id = j.user_id
GROUP BY b.segment
