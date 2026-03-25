---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT channel, session_id, is_active
    FROM smelt.ref('py_l1_465')
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
