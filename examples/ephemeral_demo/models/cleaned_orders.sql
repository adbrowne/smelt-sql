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

--- name: test_cleaned_orders ---
materialization: test
test:
  model: cleaned_orders
  inputs:
    raw_orders:
      - {order_id: 1, user_id: 100, amount: 29.99, status: completed, created_at: '2024-01-15'}
      - {order_id: 2, user_id: 101, amount: 49.99, status: completed, created_at: '2024-01-15'}
      - {order_id: 3, user_id: 100, amount: 15.00, status: cancelled, created_at: '2024-01-16'}
  expect:
    - {order_id: 1, user_id: 100, amount: 29.99, order_date: '2024-01-15'}
    - {order_id: 2, user_id: 101, amount: 49.99, order_date: '2024-01-15'}
---

