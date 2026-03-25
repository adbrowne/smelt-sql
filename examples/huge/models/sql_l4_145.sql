---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT profit, os_name, device_type
    FROM smelt.ref('sql_l3_122')
    WHERE score >= 50
)
SELECT
    b.profit,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l3_122') j ON b.user_id = j.user_id
GROUP BY b.profit
