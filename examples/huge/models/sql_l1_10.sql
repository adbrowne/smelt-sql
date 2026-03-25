---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH filtered AS (
    SELECT transaction_id, browser, os_name
    FROM smelt.ref('transactions')
    WHERE score >= 50
),
aggregated AS (
    SELECT transaction_id, COUNT(*) AS cnt
    FROM filtered
    GROUP BY transaction_id
)
SELECT
    a.transaction_id,
    a.cnt,
    f.browser
FROM aggregated a
INNER JOIN filtered f ON a.transaction_id = f.transaction_id
