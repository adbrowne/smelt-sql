---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT rating, country, updated_at
    FROM smelt.orders
    WHERE country = 'US'
),
aggregated AS (
    SELECT rating, COUNT(*) AS cnt
    FROM filtered
    GROUP BY rating
)
SELECT
    a.rating,
    a.cnt,
    f.country
FROM aggregated a
INNER JOIN filtered f ON a.rating = f.rating

