---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
WITH base AS (
    SELECT tier, rating, event_time
    FROM smelt.ref('py_l3_325')
    WHERE score >= 50
)
SELECT
    b.tier,
    SUM(amount) AS agg_val
FROM base b
INNER JOIN smelt.ref('py_l3_427') j ON b.user_id = j.user_id
GROUP BY b.tier
