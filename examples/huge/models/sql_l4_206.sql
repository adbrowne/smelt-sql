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
    SELECT event_date, rating, cost
    FROM smelt.sql_l3_103
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

