---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT channel, session_id, is_active
    FROM smelt.sql_l1_100
    WHERE quantity > 0
),
aggregated AS (
    SELECT channel, COUNT(*) AS cnt
    FROM filtered
    GROUP BY channel
)
SELECT
    a.channel,
    a.cnt,
    f.session_id
FROM aggregated a
INNER JOIN filtered f ON a.channel = f.channel

