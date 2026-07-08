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
    SELECT segment, event_date, revenue
    FROM smelt.sql_l1_191
    WHERE event_type = 'purchase'
)
SELECT
    b.segment,
    MIN(created_at) AS agg_val
FROM base b
INNER JOIN smelt.sql_l1_22 j ON b.user_id = j.user_id
GROUP BY b.segment
