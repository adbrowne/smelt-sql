--- name: user_totals ---
---
-- Per-user lifetime totals from cleaned orders.
-- Also references the ephemeral 'cleaned_orders' — it will be inlined here too.
SELECT
    user_id,
    COUNT(*) AS order_count,
    SUM(amount) AS lifetime_value
FROM smelt.models.cleaned_orders
GROUP BY user_id

--- name: test_user_totals ---
materialization: test
test:
  model: user_totals
  inputs:
    cleaned_orders:
      - {order_id: 1, user_id: 100, amount: 29.99, order_date: '2024-01-15'}
      - {order_id: 2, user_id: 101, amount: 49.99, order_date: '2024-01-15'}
      - {order_id: 3, user_id: 100, amount: 75.50, order_date: '2024-01-16'}
  expect:
    - {user_id: 100, order_count: 2, lifetime_value: 105.49}
    - {user_id: 101, order_count: 1, lifetime_value: 49.99}
---

--- name: test_user_totals_property ---
materialization: test
test:
  model: user_totals
  cases: 10
  inputs:
    cleaned_orders:
      - {user_id: 1, amount: 100.0}
      - {user_id: 1, amount: 200.0}
      - {user_id: 2, amount: 50.0}
  expect:
    - {user_id: 1, order_count: 2, lifetime_value: 300.0}
    - {user_id: 2, order_count: 1, lifetime_value: 50.0}
---

