---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT region, status, country
    FROM smelt.sql_l2_202
    WHERE created_at >= '2024-01-01'
),
aggregated AS (
    SELECT region, COUNT(*) AS cnt
    FROM filtered
    GROUP BY region
)
SELECT
    a.region,
    a.cnt,
    f.status
FROM aggregated a
INNER JOIN filtered f ON a.region = f.region

