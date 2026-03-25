---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT referrer, os_name, page_path
    FROM smelt.ref('sql_l2_144')
    WHERE amount > 0
)
SELECT
    b.referrer,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l2_311') j ON b.user_id = j.user_id
GROUP BY b.referrer
