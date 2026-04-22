-- Orders enriched with status labels (bugs #1 seed ref, #2 JOIN types)
SELECT
    o.order_id,
    o.customer_id,
    o.order_date,
    o.status AS status_code,
    os.status_label,
    os.is_terminal,
    os.is_successful,
    o.payment_method,
    o.discount_pct / 100.0 AS discount_rate
FROM smelt.source('raw.orders') AS o
LEFT JOIN smelt.ref('order_statuses') AS os ON o.status = os.status_code
