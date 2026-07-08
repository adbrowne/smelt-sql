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
    SELECT cost, page_path, platform
    FROM smelt.sql_l1_77
    WHERE event_type = 'purchase'
),
aggregated AS (
    SELECT cost, COUNT(*) AS cnt
    FROM filtered
    GROUP BY cost
)
SELECT
    a.cost,
    a.cnt,
    f.page_path
FROM aggregated a
INNER JOIN filtered f ON a.cost = f.cost
