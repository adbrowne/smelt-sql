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
    SELECT rating, quantity, is_active
    FROM smelt.sql_l3_22
    WHERE event_type = 'purchase'
),
aggregated AS (
    SELECT rating, COUNT(*) AS cnt
    FROM filtered
    GROUP BY rating
)
SELECT
    a.rating,
    a.cnt,
    f.quantity
FROM aggregated a
INNER JOIN filtered f ON a.rating = f.rating

