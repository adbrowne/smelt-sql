-- Product performance with RANK, DENSE_RANK, and LAG window functions
SELECT
    p.product_id,
    p.category,
    p.subcategory,
    p.brand_tier,
    p.unit_price,
    COUNT(DISTINCT oi.order_id) AS order_count,
    SUM(oi.quantity) AS total_units_sold,
    SUM(oi.unit_price * oi.quantity) AS total_revenue,
    RANK() OVER (
        PARTITION BY p.category
        ORDER BY SUM(oi.unit_price * oi.quantity) DESC
    ) AS category_revenue_rank,
    DENSE_RANK() OVER (
        ORDER BY SUM(oi.quantity) DESC
    ) AS global_units_rank,
    LAG(SUM(oi.unit_price * oi.quantity)) OVER (
        PARTITION BY p.category
        ORDER BY SUM(oi.unit_price * oi.quantity) DESC
    ) AS prev_product_revenue
FROM smelt.staging.stg_products AS p
LEFT JOIN smelt.staging.stg_order_items AS oi ON p.product_id = oi.product_id
GROUP BY p.product_id, p.category, p.subcategory, p.brand_tier, p.unit_price

