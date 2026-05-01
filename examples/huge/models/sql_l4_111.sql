---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT amount, segment, os_name
    FROM smelt.models.sql_l3_4
    WHERE platform = 'web'
)
SELECT
    b.amount,
    MAX(created_at) AS agg_val
FROM base b
INNER JOIN smelt.models.sql_l3_4 j ON b.user_id = j.user_id
GROUP BY b.amount

