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
    SELECT channel, referrer, amount
    FROM smelt.sql_l2_42
    WHERE amount > 0
)
SELECT
    b.channel,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.sql_l2_53 j ON b.user_id = j.user_id
GROUP BY b.channel

