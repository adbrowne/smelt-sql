---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT os_name, plan_type, event_time
    FROM smelt.sql_l3_58
    WHERE category IS NOT NULL
)
SELECT
    b.os_name,
    COUNT(*) AS agg_val
FROM base b
INNER JOIN smelt.sql_l3_177 j ON b.user_id = j.user_id
GROUP BY b.os_name
