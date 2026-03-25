---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT event_date, rating, cost
    FROM smelt.ref('py_l3_390')
    WHERE country = 'US'
),
aggregated AS (
    SELECT event_date, COUNT(*) AS cnt
    FROM filtered
    GROUP BY event_date
)
SELECT
    a.event_date,
    a.cnt,
    f.rating
FROM aggregated a
INNER JOIN filtered f ON a.event_date = f.event_date
