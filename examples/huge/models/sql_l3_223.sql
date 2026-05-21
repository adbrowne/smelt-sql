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
    SELECT platform, browser, price
    FROM smelt.sql_l2_146
    WHERE status = 'active'
)
SELECT
    b.platform,
    AVG(price) AS agg_val
FROM base b
INNER JOIN smelt.sql_l2_106 j ON b.user_id = j.user_id
GROUP BY b.platform

