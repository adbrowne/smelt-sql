-- Inventory turnover with RANGE frame and LAG/LEAD
SELECT
    p.product_id,
    p.category,
    p.subcategory,
    p.brand_tier,
    oi.item_date,
    SUM(oi.quantity) AS daily_units_sold,
    SUM(oi.unit_price * oi.quantity) AS daily_revenue,
    LAG(SUM(oi.quantity), 1) OVER (
        PARTITION BY p.product_id
        ORDER BY oi.item_date
    ) AS prev_day_units,
    LEAD(SUM(oi.quantity), 1) OVER (
        PARTITION BY p.product_id
        ORDER BY oi.item_date
    ) AS next_day_units,
    AVG(SUM(oi.quantity)) OVER (
        PARTITION BY p.product_id
        ORDER BY oi.item_date
        ROWS BETWEEN 6 PRECEDING AND CURRENT ROW
    ) AS rolling_7day_avg_units
FROM smelt.ref('stg_products') AS p
INNER JOIN smelt.ref('stg_order_items') AS oi ON p.product_id = oi.product_id
GROUP BY p.product_id, p.category, p.subcategory, p.brand_tier, oi.item_date
