-- Tests that intentionally fail: wrong expected value, missing expected row,
-- extra expected row. Each smelt.test below exercises a different failure mode.
-- All three should fail when run with `smelt test`.

-- Intentional failure: expected revenue 999.99 but actual is 300.0.
smelt.test test_wrong_expected_value AS (
    SELECT order_date AS day, COUNT(*) AS order_count, SUM(amount) AS revenue
    FROM smelt.cleaned
    GROUP BY order_date
)
PASSING cleaned AS (
    {order_id: 1, user_id: 10, amount: 100.0, order_date: '2024-01-01'},
    {order_id: 2, user_id: 20, amount: 200.0, order_date: '2024-01-01'},
    {order_id: 3, user_id: 10, amount: 50.0,  order_date: '2024-01-02'}
)
EXPECT (
    {day: '2024-01-01', order_count: 2, revenue: 999.99},
    {day: '2024-01-02', order_count: 1, revenue: 50.0}
)

-- Intentional failure: two rows in actual but only one expected (row count mismatch).
smelt.test test_missing_expected_row AS (
    SELECT order_date AS day, COUNT(*) AS order_count, SUM(amount) AS revenue
    FROM smelt.cleaned
    GROUP BY order_date
)
PASSING cleaned AS (
    {order_id: 1, user_id: 10, amount: 100.0, order_date: '2024-01-01'},
    {order_id: 2, user_id: 20, amount: 200.0, order_date: '2024-01-02'}
)
EXPECT (
    {day: '2024-01-01', order_count: 1, revenue: 100.0}
)

-- Intentional failure: one row in actual but two expected (row count mismatch).
smelt.test test_extra_expected_row AS (
    SELECT order_date AS day, COUNT(*) AS order_count, SUM(amount) AS revenue
    FROM smelt.cleaned
    GROUP BY order_date
)
PASSING cleaned AS (
    {order_id: 1, user_id: 10, amount: 100.0, order_date: '2024-01-01'}
)
EXPECT (
    {day: '2024-01-01', order_count: 1, revenue: 100.0},
    {day: '2024-01-02', order_count: 1, revenue: 50.0}
)
