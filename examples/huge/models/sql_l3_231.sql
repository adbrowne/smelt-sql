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
    SELECT transaction_id, campaign_id, created_at
    FROM smelt.sql_l2_205
    WHERE amount > 0
),
aggregated AS (
    SELECT transaction_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY transaction_id
)
SELECT
    a.transaction_id,
    a.cnt,
    f.campaign_id
FROM aggregated a
INNER JOIN filtered f ON a.transaction_id = f.transaction_id
