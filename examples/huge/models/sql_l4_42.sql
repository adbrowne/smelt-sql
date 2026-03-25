---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT price, event_type, rating
    FROM smelt.ref('py_l3_380')
    WHERE score >= 50
),
aggregated AS (
    SELECT price, COUNT(*) AS cnt
    FROM filtered
    GROUP BY price
)
SELECT
    a.price,
    a.cnt,
    f.event_type
FROM aggregated a
INNER JOIN filtered f ON a.price = f.price
