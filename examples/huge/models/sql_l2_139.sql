---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT profit, price, cost
    FROM smelt.ref('py_l1_266')
    WHERE category IS NOT NULL
)
SELECT
    b.profit,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l1_266') j ON b.user_id = j.user_id
GROUP BY b.profit
