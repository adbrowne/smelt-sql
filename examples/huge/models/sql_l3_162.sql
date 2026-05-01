---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT profit, email_domain, device_type
    FROM smelt.models.sql_l2_238
    WHERE quantity > 0
)
SELECT
    b.profit,
    AVG(price) AS agg_val
FROM base b
INNER JOIN smelt.models.sql_l2_39 j ON b.user_id = j.user_id
GROUP BY b.profit

