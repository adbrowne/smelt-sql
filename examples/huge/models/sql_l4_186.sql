---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT profit, created_at, campaign_id
    FROM smelt.models.sql_l3_100
    WHERE score >= 50
)
SELECT
    b.profit,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.models.sql_l3_180 j ON b.user_id = j.user_id
GROUP BY b.profit

