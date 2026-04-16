-- Orders enriched via subquery ref (bug #6 subquery ref replacement)
SELECT
    sub.order_id,
    sub.customer_id,
    sub.order_date,
    sub.status_code,
    sub.status_label,
    sub.discount_rate
FROM (
    SELECT
        order_id,
        customer_id,
        order_date,
        status_code,
        status_label,
        discount_rate
    FROM smelt.ref('stg_orders')
) AS sub
