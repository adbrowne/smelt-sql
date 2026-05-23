---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT referrer, os_name, page_path
    FROM smelt.sql_l2_27
    WHERE amount > 0
)
SELECT
    b.referrer,
    SUM(revenue) AS agg_val
FROM base b
INNER JOIN smelt.sql_l2_77 j ON b.user_id = j.user_id
GROUP BY b.referrer
