-- Aggregates cleaned orders into daily revenue.
-- The ephemeral ref to 'cleaned_orders' will be inlined as a CTE.
SELECT
    order_date,
    COUNT(*) AS order_count,
    SUM(amount) AS total_revenue
FROM smelt.ref('cleaned_orders')
GROUP BY order_date
