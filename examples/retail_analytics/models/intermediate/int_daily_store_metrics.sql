-- Daily store metrics with date-based grouping
SELECT
    o.order_date,
    o.store_id,
    o.store_type,
    o.store_region,
    COUNT(DISTINCT o.order_id) AS order_count,
    COUNT(DISTINCT o.customer_id) AS unique_customers,
    SUM(o.gross_revenue) AS gross_revenue,
    SUM(o.net_revenue) AS net_revenue,
    SUM(o.total_quantity) AS total_units,
    SUM(o.gross_revenue) / COUNT(DISTINCT o.order_id) AS avg_order_value
FROM smelt.intermediate.int_order_enriched AS o
GROUP BY o.order_date, o.store_id, o.store_type, o.store_region

