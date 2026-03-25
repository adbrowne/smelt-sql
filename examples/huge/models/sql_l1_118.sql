---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT session_id, event_time, region
    FROM smelt.ref('logs')
    WHERE event_type = 'purchase'
),
aggregated AS (
    SELECT session_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY session_id
)
SELECT
    a.session_id,
    a.cnt,
    f.event_time
FROM aggregated a
INNER JOIN filtered f ON a.session_id = f.session_id
