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
    SELECT browser, channel, user_id
    FROM smelt.sql_l1_80
    WHERE event_type = 'purchase'
)
SELECT
    b.browser,
    COUNT(*) AS agg_val
FROM base b
INNER JOIN smelt.sql_l1_135 j ON b.user_id = j.user_id
GROUP BY b.browser
