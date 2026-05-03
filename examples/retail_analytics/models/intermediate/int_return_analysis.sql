-- Return analysis with LEFT JOIN and FILTER clause on aggregates
SELECT
    p.category,
    p.subcategory,
    p.brand_tier,
    COUNT(*) AS total_items_sold,
    COUNT(*) FILTER (WHERE oi.is_returned = true) AS returned_items,
    CAST(COUNT(*) FILTER (WHERE oi.is_returned = true) AS FLOAT)
        / CAST(COUNT(*) AS FLOAT) AS return_rate,
    COUNT(DISTINCT oi.return_reason) FILTER (WHERE oi.is_returned = true) AS distinct_return_reasons,
    SUM(oi.unit_price * oi.quantity) AS total_revenue,
    SUM(oi.unit_price * oi.quantity) FILTER (WHERE oi.is_returned = true) AS returned_revenue
FROM smelt.staging.stg_order_items AS oi
LEFT JOIN smelt.staging.stg_products AS p ON oi.product_id = p.product_id
GROUP BY p.category, p.subcategory, p.brand_tier

