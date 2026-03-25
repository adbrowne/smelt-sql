---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT event_time, referrer, duration_seconds
    FROM smelt.ref('sql_l2_245')
    WHERE score >= 50
),
aggregated AS (
    SELECT event_time, COUNT(*) AS cnt
    FROM filtered
    GROUP BY event_time
)
SELECT
    a.event_time,
    a.cnt,
    f.referrer
FROM aggregated a
INNER JOIN filtered f ON a.event_time = f.event_time
