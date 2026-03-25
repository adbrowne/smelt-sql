---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT rating, country, updated_at
    FROM smelt.ref('orders')
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
