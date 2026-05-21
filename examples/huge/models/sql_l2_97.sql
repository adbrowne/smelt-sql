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
    SELECT event_date, page_path, ip_address
    FROM smelt.sql_l1_242
    WHERE event_type = 'purchase'
)
SELECT
    b.event_date,
    AVG(price) AS agg_val
FROM base b
INNER JOIN smelt.sql_l1_242 j ON b.user_id = j.user_id
GROUP BY b.event_date

