--- name: daily_revenue ---
columns:
  order_date:
    tests: [not_null]
  order_count:
    tests: [not_null, {min: 1}]
  total_revenue:
    tests: [not_null, {min: 0}]
---
-- Aggregates cleaned orders into daily revenue.
-- The ephemeral ref to 'cleaned_orders' will be inlined as a CTE.
SELECT
    order_date,
    COUNT(*) AS order_count,
    SUM(amount) AS total_revenue
FROM smelt.models.cleaned_orders
GROUP BY order_date

--- name: test_daily_revenue ---
materialization: test
test:
  model: daily_revenue
  inputs:
    cleaned_orders:
      - {order_id: 1, user_id: 100, amount: 29.99, order_date: '2024-01-15'}
      - {order_id: 2, user_id: 101, amount: 49.99, order_date: '2024-01-15'}
      - {order_id: 3, user_id: 102, amount: 75.50, order_date: '2024-01-16'}
  expect:
    - {order_date: '2024-01-15', order_count: 2, total_revenue: 79.98}
    - {order_date: '2024-01-16', order_count: 1, total_revenue: 75.5}
---

