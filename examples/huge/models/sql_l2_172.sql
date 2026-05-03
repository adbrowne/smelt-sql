---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT discount, event_time, rating
    FROM smelt.sql_l1_24
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

