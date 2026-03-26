-- Per-user lifetime totals from cleaned orders.
-- Also references the ephemeral 'cleaned_orders' — it will be inlined here too.
SELECT
    user_id,
    COUNT(*) AS order_count,
    SUM(amount) AS lifetime_value
FROM smelt.ref('cleaned_orders')
GROUP BY user_id
