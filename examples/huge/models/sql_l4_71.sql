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
    SELECT cost, platform, transaction_id
    FROM smelt.sql_l3_124
    WHERE status = 'active'
)
SELECT
    b.cost,
    AVG(duration_seconds) AS agg_val
FROM base b
INNER JOIN smelt.sql_l3_43 j ON b.user_id = j.user_id
GROUP BY b.cost
