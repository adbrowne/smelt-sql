---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH filtered AS (
    SELECT campaign_id, is_active, segment
    FROM smelt.sql_l3_176
    WHERE category IS NOT NULL
),
aggregated AS (
    SELECT campaign_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY campaign_id
)
SELECT
    a.campaign_id,
    a.cnt,
    f.is_active
FROM aggregated a
INNER JOIN filtered f ON a.campaign_id = f.campaign_id
