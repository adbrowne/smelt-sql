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
    SELECT channel, profit, category
    FROM smelt.sql_l2_73
    WHERE category IS NOT NULL
)
SELECT
    b.channel,
    COUNT(*) AS agg_val
FROM base b
INNER JOIN smelt.sql_l2_184 j ON b.user_id = j.user_id
GROUP BY b.channel
