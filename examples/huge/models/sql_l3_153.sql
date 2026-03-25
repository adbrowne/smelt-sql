---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT channel, is_verified, category
    FROM smelt.ref('sql_l2_119')
    WHERE status = 'active'
),
aggregated AS (
    SELECT channel, COUNT(*) AS cnt
    FROM filtered
    GROUP BY channel
)
SELECT
    a.channel,
    a.cnt,
    f.is_verified
FROM aggregated a
INNER JOIN filtered f ON a.channel = f.channel
