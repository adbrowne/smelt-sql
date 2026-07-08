---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT rating, plan_type, event_time
    FROM smelt.sql_l1_17
    WHERE score >= 50
),
aggregated AS (
    SELECT rating, COUNT(*) AS cnt
    FROM filtered
    GROUP BY rating
)
SELECT
    a.rating,
    a.cnt,
    f.plan_type
FROM aggregated a
INNER JOIN filtered f ON a.rating = f.rating
