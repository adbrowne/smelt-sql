smelt.check no_negative_amounts AS (
    SELECT order_id, amount
    FROM smelt.revenue
    WHERE amount < 0
)
