---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT user_id, category, plan_type
    FROM smelt.sql_l3_12
    WHERE score >= 50
),
aggregated AS (
    SELECT user_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY user_id
)
SELECT
    a.user_id,
    a.cnt,
    f.category
FROM aggregated a
INNER JOIN filtered f ON a.user_id = f.user_id
