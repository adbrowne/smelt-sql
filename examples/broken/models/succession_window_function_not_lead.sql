---
materialization: table
refresh: incremental
---
-- SuccessionWindowFunctionNotLead: the projected window is SUM(...) OVER,
-- not LEAD(t)/LAG(t) at the default offset
-- (`docs/specs/model_properties.md` §"Keyed-succession classification").
SELECT
    customer_id,
    changed_at,
    SUM(customer_id) OVER (PARTITION BY customer_id ORDER BY changed_at) AS total
FROM smelt.sources.succession_changes
