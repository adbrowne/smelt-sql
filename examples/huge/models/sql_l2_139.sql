---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT profit, price, cost
    FROM smelt.models.sql_l1_62
    WHERE category IS NOT NULL
)
SELECT
    b.profit,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.models.sql_l1_62 j ON b.user_id = j.user_id
GROUP BY b.profit

