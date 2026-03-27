-- A simple model with CTE chain, used as target for failing tests below.
--- name: revenue_summary ---
---
WITH cleaned AS (
    SELECT
        order_id,
        user_id,
        amount,
        CAST(order_date AS DATE) AS order_date
    FROM raw_data
    WHERE status = 'completed'
),
daily AS (
    SELECT
        order_date AS day,
        COUNT(*) AS order_count,
        SUM(amount) AS revenue
    FROM cleaned
    GROUP BY order_date
)
SELECT * FROM daily

--- name: test_wrong_expected_value ---
materialization: test
test:
  model: revenue_summary
  target_cte: daily
  inputs:
    cleaned:
      - {order_id: 1, user_id: 10, amount: 100.0, order_date: '2024-01-01'}
      - {order_id: 2, user_id: 20, amount: 200.0, order_date: '2024-01-01'}
      - {order_id: 3, user_id: 10, amount: 50.0, order_date: '2024-01-02'}
  expect:
    - {day: '2024-01-01', order_count: 2, revenue: 999.99}
    - {day: '2024-01-02', order_count: 1, revenue: 50.0}
---

--- name: test_missing_expected_row ---
materialization: test
test:
  model: revenue_summary
  target_cte: daily
  inputs:
    cleaned:
      - {order_id: 1, user_id: 10, amount: 100.0, order_date: '2024-01-01'}
      - {order_id: 2, user_id: 20, amount: 200.0, order_date: '2024-01-02'}
  expect:
    - {day: '2024-01-01', order_count: 1, revenue: 100.0}
---

--- name: test_extra_expected_row ---
materialization: test
test:
  model: revenue_summary
  target_cte: daily
  inputs:
    cleaned:
      - {order_id: 1, user_id: 10, amount: 100.0, order_date: '2024-01-01'}
  expect:
    - {day: '2024-01-01', order_count: 1, revenue: 100.0}
    - {day: '2024-01-02', order_count: 1, revenue: 50.0}
---
