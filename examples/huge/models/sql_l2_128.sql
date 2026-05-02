---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT channel, status, profit
    FROM smelt.models.sql_l1_147
    WHERE event_type = 'purchase'
)
SELECT
    b.channel,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.models.sql_l1_185 j ON b.user_id = j.user_id
GROUP BY b.channel

