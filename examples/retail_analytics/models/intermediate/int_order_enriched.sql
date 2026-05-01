-- Orders enriched with customer, store, and item aggregates (multi-table JOIN)
SELECT
    o.order_id,
    o.order_date,
    o.order_status,
    o.payment_method,
    o.shipping_method,
    o.discount_rate,
    c.customer_id,
    c.segment AS customer_segment,
    c.country AS customer_country,
    s.store_id,
    s.store_type,
    s.region AS store_region,
    COUNT(oi.order_item_id) AS item_count,
    SUM(oi.quantity) AS total_quantity,
    SUM(oi.unit_price * oi.quantity) AS gross_revenue,
    SUM(oi.unit_price * oi.quantity) * (1.0 - o.discount_rate) AS net_revenue
FROM smelt.models.staging.stg_orders AS o
INNER JOIN smelt.models.staging.stg_customers AS c ON o.customer_id = c.customer_id
INNER JOIN smelt.models.staging.stg_stores AS s ON o.store_id = s.store_id
LEFT JOIN smelt.models.staging.stg_order_items AS oi ON o.order_id = oi.order_id
GROUP BY
    o.order_id, o.order_date, o.order_status, o.payment_method,
    o.shipping_method, o.discount_rate,
    c.customer_id, c.segment, c.country,
    s.store_id, s.store_type, s.region

