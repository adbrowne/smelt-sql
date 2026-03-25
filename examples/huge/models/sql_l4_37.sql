---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT status, revenue, referrer
    FROM smelt.ref('sql_l3_60')
    WHERE country = 'US'
)
SELECT
    b.status,
    AVG(price) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l3_89') j ON b.user_id = j.user_id
GROUP BY b.status
