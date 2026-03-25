---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT discount, event_time, rating
    FROM smelt.ref('sql_l1_247')
    WHERE country = 'US'
),
aggregated AS (
    SELECT discount, COUNT(*) AS cnt
    FROM filtered
    GROUP BY discount
)
SELECT
    a.discount,
    a.cnt,
    f.event_time
FROM aggregated a
INNER JOIN filtered f ON a.discount = f.discount
