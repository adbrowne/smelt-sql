smelt.check amount_must_exceed_500 AS (
    SELECT order_id, amount FROM smelt.revenue WHERE amount < 500
)
