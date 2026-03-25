---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT status, region, transaction_id
    FROM smelt.ref('py_l3_416')
    WHERE country = 'US'
)
SELECT
    b.status,
    SUM(quantity) AS agg_val
FROM base b
INNER JOIN smelt.ref('sql_l3_2') j ON b.user_id = j.user_id
GROUP BY b.status
