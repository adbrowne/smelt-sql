-- Products enriched with category hierarchy (bugs #1 seed ref, #2 JOIN types, #3 CASE)
SELECT
    p.product_id,
    p.product_name,
    p.category_code,
    ch.category_name,
    ch.department,
    p.brand_tier,
    p.unit_price_cents / 100.0 AS unit_price,
    p.cost_cents / 100.0 AS unit_cost,
    p.weight_grams / 1000.0 AS weight_kg,
    CASE WHEN p.is_digital THEN 'Digital' ELSE 'Physical' END AS product_type
FROM smelt.source('raw.products') AS p
LEFT JOIN smelt.ref('category_hierarchy') AS ch ON p.category_code = ch.category_code
