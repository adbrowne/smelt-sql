---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT event_time, cost, score
    FROM smelt.ref('sql_l1_26')
    WHERE quantity > 0
)
SELECT
    b.event_time,
    AVG(price) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_26') j ON b.user_id = j.user_id
GROUP BY b.event_time
