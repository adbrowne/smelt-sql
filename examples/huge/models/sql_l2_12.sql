---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT event_time, cost, score
    FROM smelt.sql_l1_183
    WHERE quantity > 0
)
SELECT
    b.event_time,
    AVG(price) AS agg_val
FROM base b
INNER JOIN smelt.sql_l1_183 j ON b.user_id = j.user_id
GROUP BY b.event_time

