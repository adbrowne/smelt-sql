--- name: order_analysis ---
materialization: view
---
-- Multi-CTE model used by the new-syntax smelt.test fixtures.
-- 'daily' groups completed orders by date; 'ranked' adds a rank per-date.
WITH daily AS (
    SELECT
        created_at AS order_date,
        COUNT(*) AS order_count,
        SUM(amount) AS daily_total
    FROM smelt.raw_orders
    WHERE status = 'completed'
    GROUP BY created_at
),
ranked AS (
    SELECT
        order_date,
        order_count,
        daily_total,
        ROW_NUMBER() OVER (ORDER BY daily_total DESC) AS revenue_rank
    FROM daily
)
SELECT * FROM ranked
