---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT region, user_id, referrer
    FROM smelt.ref('sql_l2_17')
    WHERE is_active = true
)
SELECT
    b.region,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l2_109') j ON b.user_id = j.user_id
GROUP BY b.region
