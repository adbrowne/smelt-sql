---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT is_verified, plan_type, region
    FROM smelt.sql_l1_143
    WHERE status = 'active'
)
SELECT
    b.is_verified,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.sql_l1_66 j ON b.user_id = j.user_id
GROUP BY b.is_verified

