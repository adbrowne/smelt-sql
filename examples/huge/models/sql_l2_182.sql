---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT is_active, discount, duration_seconds
    FROM smelt.sql_l1_28
    WHERE country = 'US'
),
aggregated AS (
    SELECT is_active, COUNT(*) AS cnt
    FROM filtered
    GROUP BY is_active
)
SELECT
    a.is_active,
    a.cnt,
    f.discount
FROM aggregated a
INNER JOIN filtered f ON a.is_active = f.is_active

