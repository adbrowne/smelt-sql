---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT tier, rating, event_time
    FROM smelt.sql_l3_149
    WHERE score >= 50
)
SELECT
    b.tier,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.sql_l3_149 j ON b.user_id = j.user_id
GROUP BY b.tier

