---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT discount, event_time, rating
    FROM smelt.ref('sql_l3_212')
    WHERE quantity > 0
)
SELECT
    b.discount,
    AVG(duration_seconds) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l3_419') j ON b.user_id = j.user_id
GROUP BY b.discount
