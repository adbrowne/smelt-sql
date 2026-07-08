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
    SELECT event_date, event_type, score
    FROM smelt.sql_l2_240
    WHERE event_type = 'purchase'
)
SELECT
    b.event_date,
    COUNT(*) AS agg_val
FROM base b
INNER JOIN smelt.sql_l2_212 j ON b.user_id = j.user_id
GROUP BY b.event_date
