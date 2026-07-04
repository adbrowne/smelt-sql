---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT created_at, country, session_id
    FROM smelt.sql_l2_57
    WHERE amount > 0
),
aggregated AS (
    SELECT created_at, COUNT(*) AS cnt
    FROM filtered
    GROUP BY created_at
)
SELECT
    a.created_at,
    a.cnt,
    f.country
FROM aggregated a
INNER JOIN filtered f ON a.created_at = f.created_at
