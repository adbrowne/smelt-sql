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
    SELECT profit, country, category
    FROM smelt.sql_l2_6
    WHERE country = 'US'
)
SELECT
    b.profit,
    COUNT(*) AS agg_val
FROM base b
INNER JOIN smelt.sql_l2_17 j ON b.user_id = j.user_id
GROUP BY b.profit
