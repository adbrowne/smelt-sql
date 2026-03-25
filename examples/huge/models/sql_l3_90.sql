---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT channel, referrer, amount
    FROM smelt.ref('py_l2_491')
    WHERE amount > 0
)
SELECT
    b.channel,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l2_401') j ON b.user_id = j.user_id
GROUP BY b.channel
