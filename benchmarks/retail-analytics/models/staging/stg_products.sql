-- Staged product dimension with price conversion
SELECT
    product_id,
    category,
    subcategory,
    brand_tier,
    CAST(unit_price_cents AS FLOAT) / 100.0 AS unit_price,
    CAST(weight_grams AS FLOAT) / 1000.0 AS weight_kg,
    CASE WHEN is_digital THEN 'Digital' ELSE 'Physical' END AS product_type
FROM smelt.source('raw.products')
