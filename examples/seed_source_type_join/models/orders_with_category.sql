---
materialization: view
---
-- D5 probe: joins seed (inferred types) with source (declared aliases).
-- The seed categories.csv has category_id INTEGER (from alias INT in sidecar),
-- the source orders has category_id INT (= INTEGER). Join should be type-clean.
-- discount_pct is DECIMAL(4,2); amount is DECIMAL(10,2); their product is DECIMAL.
SELECT
    o.order_id,
    o.category_id,
    c.category_name,
    o.amount,
    c.discount_pct,
    o.amount * (1.0 - c.discount_pct) AS discounted_amount,
    c.is_active
FROM smelt.sources.raw.orders o
JOIN smelt.categories c ON o.category_id = c.category_id
WHERE c.is_active = true
