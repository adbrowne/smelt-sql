---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT referrer, amount, os_name
    FROM smelt.ref('sql_l1_39')
    WHERE is_active = true
)
SELECT
    b.referrer,
    COUNT(DISTINCT user_id) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l1_49') j ON b.user_id = j.user_id
GROUP BY b.referrer
