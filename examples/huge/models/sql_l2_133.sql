---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT transaction_id, platform, is_verified
    FROM smelt.ref('py_l1_458')
    WHERE event_type = 'purchase'
),
aggregated AS (
    SELECT transaction_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY transaction_id
)
SELECT
    a.transaction_id,
    a.cnt,
    f.platform
FROM aggregated a
INNER JOIN filtered f ON a.transaction_id = f.transaction_id
