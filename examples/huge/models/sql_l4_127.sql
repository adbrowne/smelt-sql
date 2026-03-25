---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT event_time, event_date, event_type
    FROM smelt.ref('py_l3_289')
    WHERE country = 'US'
),
aggregated AS (
    SELECT event_time, COUNT(*) AS cnt
    FROM filtered
    GROUP BY event_time
)
SELECT
    a.event_time,
    a.cnt,
    f.event_date
FROM aggregated a
INNER JOIN filtered f ON a.event_time = f.event_time
