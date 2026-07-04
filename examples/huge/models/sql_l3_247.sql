---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT event_time, score, plan_type
    FROM smelt.sql_l2_120
    WHERE category IS NOT NULL
)
SELECT
    b.event_time,
    MIN(created_at) AS agg_val
FROM base b
INNER JOIN smelt.sql_l2_16 j ON b.user_id = j.user_id
GROUP BY b.event_time
