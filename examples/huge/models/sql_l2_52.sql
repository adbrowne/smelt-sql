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
    SELECT channel, campaign_id, region
    FROM smelt.sql_l1_132
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

