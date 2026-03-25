---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT channel, campaign_id, region
    FROM smelt.ref('sql_l1_158')
    WHERE amount > 0
),
aggregated AS (
    SELECT channel, COUNT(*) AS cnt
    FROM filtered
    GROUP BY channel
)
SELECT
    a.channel,
    a.cnt,
    f.campaign_id
FROM aggregated a
INNER JOIN filtered f ON a.channel = f.channel
