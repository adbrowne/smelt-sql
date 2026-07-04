---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT plan_type, discount, session_id
    FROM smelt.sql_l3_219
    WHERE quantity > 0
)
SELECT
    b.plan_type,
    MAX(created_at) AS agg_val
FROM base b
INNER JOIN smelt.sql_l3_219 j ON b.user_id = j.user_id
GROUP BY b.plan_type
