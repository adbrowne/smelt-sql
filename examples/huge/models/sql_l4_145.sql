---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT profit, os_name, device_type
    FROM smelt.sql_l3_83
    WHERE score >= 50
)
SELECT
    b.profit,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.sql_l3_83 j ON b.user_id = j.user_id
GROUP BY b.profit

