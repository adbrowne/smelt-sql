smelt.check daily_revenue_non_negative AS (
    SELECT order_id, amount
    FROM smelt.raw_orders
    WHERE amount < 0
)
