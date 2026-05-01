---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT discount, event_time, rating
    FROM smelt.models.sql_l3_205
    WHERE quantity > 0
)
SELECT
    b.discount,
    AVG(duration_seconds) AS agg_val
FROM base b
INNER JOIN smelt.models.sql_l3_205 j ON b.user_id = j.user_id
GROUP BY b.discount

