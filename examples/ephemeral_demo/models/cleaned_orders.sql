--- name: cleaned_orders ---
materialization: ephemeral
---
-- Ephemeral model: filters to completed orders and normalizes columns.
-- This is never materialized — it gets inlined as a CTE into downstream models.
SELECT
    order_id,
    user_id,
    amount,
    created_at AS order_date
FROM smelt.raw_orders
WHERE status = 'completed'
