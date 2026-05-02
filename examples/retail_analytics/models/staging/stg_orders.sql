-- Staged orders with date casting and status normalization
SELECT
    order_id,
    customer_id,
    store_id,
    CAST(order_date AS DATE) AS order_date,
    CASE
        WHEN status IN ('completed', 'shipped') THEN 'fulfilled'
        WHEN status = 'processing' THEN 'pending'
        WHEN status = 'cancelled' THEN 'cancelled'
        WHEN status = 'returned' THEN 'returned'
        ELSE 'unknown'
    END AS order_status,
    COALESCE(payment_method, 'unknown') AS payment_method,
    COALESCE(shipping_method, 'standard') AS shipping_method,
    CAST(discount_pct AS FLOAT) / 100.0 AS discount_rate
FROM smelt.sources.raw.orders

