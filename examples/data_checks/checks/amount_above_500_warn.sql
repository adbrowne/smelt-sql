---
severity: warn
---
smelt.check amount_above_500_warn AS (
    SELECT order_id, amount FROM smelt.revenue WHERE amount < 500
)
