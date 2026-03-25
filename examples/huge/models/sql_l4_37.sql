---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT status, revenue, referrer
    FROM smelt.ref('sql_l3_87')
    WHERE country = 'US'
)
SELECT
    b.status,
    AVG(price) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l3_343') j ON b.user_id = j.user_id
GROUP BY b.status
