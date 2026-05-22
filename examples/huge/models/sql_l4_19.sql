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
    SELECT status, region, transaction_id
    FROM smelt.sql_l3_242
    WHERE country = 'US'
)
SELECT
    b.status,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.sql_l3_175 j ON b.user_id = j.user_id
GROUP BY b.status
