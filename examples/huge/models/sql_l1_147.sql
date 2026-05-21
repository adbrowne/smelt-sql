---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT page_path, rating, updated_at
    FROM smelt.refunds
    WHERE event_type = 'purchase'
),
aggregated AS (
    SELECT page_path, COUNT(*) AS cnt
    FROM filtered
    GROUP BY page_path
)
SELECT
    a.page_path,
    a.cnt,
    f.rating
FROM aggregated a
INNER JOIN filtered f ON a.page_path = f.page_path

