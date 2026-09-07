---
refresh: incremental
---
SELECT
  customer_id,
  tier,
  region,
  effective_ts AS valid_from,
  LEAD(effective_ts) OVER (PARTITION BY customer_id ORDER BY effective_ts) AS valid_to,
  LEAD(effective_ts) OVER (PARTITION BY customer_id ORDER BY effective_ts) IS NULL AS is_current
FROM smelt.sources.customer_changes
QUALIFY NOT is_deleted
