SELECT
    s.customer_id,
    COUNT(*) AS invoice_count,
    SUM(s.amount) AS total_amount
FROM smelt.staging.stg_invoices s
GROUP BY s.customer_id
