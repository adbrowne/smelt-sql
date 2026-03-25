---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT score, cost, duration_seconds
    FROM smelt.ref('reviews')
    WHERE score >= 50
),
aggregated AS (
    SELECT score, COUNT(*) AS cnt
    FROM filtered
    GROUP BY score
)
SELECT
    a.score,
    a.cnt,
    f.cost
FROM aggregated a
INNER JOIN filtered f ON a.score = f.score
