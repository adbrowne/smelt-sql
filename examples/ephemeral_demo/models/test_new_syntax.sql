-- New-syntax smelt.test declarations (Phase 5).
--
-- These exercise the AST-driven test runner: PASSING mocks external deps,
-- EXPECT provides expected rows, and #cte targets a CTE within a model.

smelt.test check_completed_orders AS (
    SELECT
        COUNT(*) AS order_count,
        SUM(amount) AS daily_total
    FROM smelt.raw_orders
    WHERE status = 'completed'
)
PASSING raw_orders AS (
    {order_id: 1, user_id: 100, amount: 100, status: 'completed', created_at: '2024-01-15'},
    {order_id: 2, user_id: 101, amount: 200, status: 'completed', created_at: '2024-01-15'},
    {order_id: 3, user_id: 102, amount: 75,  status: 'cancelled', created_at: '2024-01-16'},
    {order_id: 4, user_id: 103, amount: 50,  status: 'completed', created_at: '2024-01-16'}
)
EXPECT (
    {order_count: 3, daily_total: 350}
)

smelt.test check_daily_cte AS (
    SELECT order_date, order_count, daily_total
    FROM smelt.order_analysis#daily
)
PASSING raw_orders AS (
    {order_id: 1, user_id: 100, amount: 100, status: 'completed', created_at: '2024-01-15'},
    {order_id: 2, user_id: 101, amount: 200, status: 'completed', created_at: '2024-01-15'},
    {order_id: 3, user_id: 102, amount: 75,  status: 'cancelled', created_at: '2024-01-16'},
    {order_id: 4, user_id: 103, amount: 50,  status: 'completed', created_at: '2024-01-16'}
)
EXPECT (
    {order_date: '2024-01-15', order_count: 2, daily_total: 300},
    {order_date: '2024-01-16', order_count: 1, daily_total: 50}
)
