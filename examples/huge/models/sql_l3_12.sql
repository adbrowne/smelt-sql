---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT region, transaction_id, page_path
    FROM smelt.ref('sql_l2_16')
    WHERE created_at >= '2024-01-01'
),
aggregated AS (
    SELECT region, COUNT(*) AS cnt
    FROM filtered
    GROUP BY region
)
SELECT
    a.region,
    a.cnt,
    f.transaction_id
FROM aggregated a
INNER JOIN filtered f ON a.region = f.region
