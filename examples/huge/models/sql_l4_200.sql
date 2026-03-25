---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT rating, quantity, is_active
    FROM smelt.ref('sql_l3_181')
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
