---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT country, campaign_id, status
    FROM smelt.sql_l3_80
    WHERE platform = 'web'
),
aggregated AS (
    SELECT country, COUNT(*) AS cnt
    FROM filtered
    GROUP BY country
)
SELECT
    a.country,
    a.cnt,
    f.campaign_id
FROM aggregated a
INNER JOIN filtered f ON a.country = f.country
