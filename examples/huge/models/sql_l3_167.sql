---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT plan_type, score, is_verified
    FROM smelt.models.sql_l2_115
    WHERE event_type = 'purchase'
)
SELECT
    b.plan_type,
    AVG(amount) AS agg_val
FROM base b
INNER JOIN smelt.models.sql_l2_43 j ON b.user_id = j.user_id
GROUP BY b.plan_type

