---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT channel, status, profit
    FROM smelt.ref('sql_l1_150')
    WHERE event_type = 'purchase'
)
SELECT
    b.channel,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_150') j ON b.user_id = j.user_id
GROUP BY b.channel
