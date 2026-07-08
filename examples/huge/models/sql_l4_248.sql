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
    SELECT segment, page_path, email_domain
    FROM smelt.sql_l3_92
    WHERE created_at >= '2024-01-01'
),
aggregated AS (
    SELECT segment, COUNT(*) AS cnt
    FROM filtered
    GROUP BY segment
)
SELECT
    a.segment,
    a.cnt,
    f.page_path
FROM aggregated a
INNER JOIN filtered f ON a.segment = f.segment
