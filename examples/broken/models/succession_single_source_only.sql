---
materialization: table
refresh: incremental
---
-- SuccessionSingleSourceOnly: the FROM clause joins a second source — a
-- join is not this shape (`docs/specs/model_properties.md` §"Keyed-
-- succession classification").
SELECT
    c.customer_id,
    c.changed_at,
    LEAD(c.changed_at) OVER (PARTITION BY c.customer_id ORDER BY c.changed_at) AS next_changed_at
FROM smelt.sources.succession_changes c
JOIN smelt.sources.maintenance_orders o ON c.customer_id = o.customer_id
